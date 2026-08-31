// SettingsView.tsx — 설정 화면(좌측 네비 "설정", 이전엔 "곧" placeholder였다). Flutter SDK 경로/언어/
// 테마/빌드 완료 알림/서명키 보관함 위치/정보(About) 6개 섹션. 필드 변경 시 즉시 저장한다(별도 "저장"
// 버튼 없음) — useSettings::updateSettings 가 낙관적 갱신 + 백엔드 저장을 한 번에 한다.
//
// ⚠️ 이 화면의 구조적 라벨(섹션 제목/버튼/힌트)이 이번 i18n 범위의 핵심이다(t() 로 번역) — 다른 화면
// (프로젝트 카드 등)은 아직 한국어 고정이다(settings-context.tsx 문서 참고). 이 화면 안에서도 동적
// 에러/상태 문구(예: "Flutter를 찾지 못했어요")는 앱 전체의 다른 에러 배너들과 마찬가지로 한국어
// 고정이다 — 구조적 라벨만 번역 대상이다.

import { useEffect, useState } from "react";
import {
  checkFlutterPath,
  detectFlutterSdk,
  getAppVersion,
  getCliManifest,
  getKeystoreVaultPath,
  openExternalUrl,
  openKeystoreVault,
} from "../lib/api";
import { useSettings } from "../lib/settings-context";
import type { CliCommandDoc, Language, ThemePreference } from "../lib/types";
import { CheckStatusIcon, SpinnerIcon } from "./Icons";

const THEME_OPTIONS: ThemePreference[] = ["system", "light", "dark"];

export function SettingsView() {
  const { settings, loaded, updateSettings, t } = useSettings();

  const themeLabels: Record<ThemePreference, string> = {
    system: t("settings.theme.system"),
    light: t("settings.theme.light"),
    dark: t("settings.theme.dark"),
  };

  // ── Flutter SDK 경로 ──
  const [flutterPathInput, setFlutterPathInput] = useState("");
  const [detecting, setDetecting] = useState(false);
  const [checking, setChecking] = useState(false);
  const [versionLine, setVersionLine] = useState<string | null>(null);
  const [flutterError, setFlutterError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  const runCheck = async (path: string) => {
    setChecking(true);
    setFlutterError(null);
    try {
      const line = await checkFlutterPath(path);
      setVersionLine(line);
    } catch (e) {
      setVersionLine(null);
      setFlutterError(typeof e === "string" ? e : "Flutter 경로를 확인하지 못했어요.");
    } finally {
      setChecking(false);
    }
  };

  // 설정이 로드된(loaded === true) 직후 딱 한 번, 저장된 경로로 입력칸 + 검증 표시를 맞춘다 — loaded
  // 가 true 로 바뀌기 전에는 settings 가 아직 기본값(빈 경로)이라 이 시점에 맞춰야 한다
  // (settings-context.tsx 문서 참고).
  useEffect(() => {
    if (!loaded) return;
    setFlutterPathInput(settings.flutterPath ?? "");
    if (settings.flutterPath) void runCheck(settings.flutterPath);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded]);

  const handleDetect = async () => {
    if (detecting) return;
    setDetecting(true);
    setFlutterError(null);
    setSaveError(null);
    try {
      const found = await detectFlutterSdk();
      if (!found) {
        setFlutterError("Flutter를 찾지 못했어요. 직접 경로를 입력해 주세요.");
        return;
      }
      setFlutterPathInput(found);
      try {
        await updateSettings({ flutterPath: found });
      } catch (e) {
        setSaveError(typeof e === "string" ? e : "설정을 저장하지 못했어요.");
      }
      await runCheck(found);
    } catch (e) {
      setFlutterError(typeof e === "string" ? e : "Flutter를 찾는 중 문제가 발생했어요.");
    } finally {
      setDetecting(false);
    }
  };

  const handleFlutterPathBlur = async () => {
    const trimmed = flutterPathInput.trim();
    const normalized = trimmed.length > 0 ? trimmed : null;
    setSaveError(null);
    if (normalized !== settings.flutterPath) {
      try {
        await updateSettings({ flutterPath: normalized });
      } catch (e) {
        setSaveError(typeof e === "string" ? e : "설정을 저장하지 못했어요.");
      }
    }
    if (trimmed) {
      void runCheck(trimmed);
    } else {
      setVersionLine(null);
      setFlutterError(null);
    }
  };

  // ── 서명키 보관함 위치 ──
  const [vaultPath, setVaultPath] = useState<string | null>(null);
  const [vaultError, setVaultError] = useState<string | null>(null);
  useEffect(() => {
    getKeystoreVaultPath()
      .then(setVaultPath)
      .catch((e) => setVaultError(typeof e === "string" ? e : "보관함 위치를 확인하지 못했어요."));
  }, []);
  const handleOpenVault = async () => {
    setVaultError(null);
    try {
      await openKeystoreVault();
    } catch (e) {
      setVaultError(typeof e === "string" ? e : "Finder에서 열지 못했어요.");
    }
  };

  // ── CLI / 자동화 ── manifest 는 build.rs::cli_manifest() 가 화면과 clap --help 양쪽의 단일 소스다
  // (commands.rs::get_cli_manifest). description/example 은 원문(한국어) 그대로 표시한다 — 이 화면의
  // i18n 범위는 섹션 제목/안내문 같은 구조적 라벨뿐이다(파일 상단 문서 참고).
  const [cliManifest, setCliManifest] = useState<CliCommandDoc[]>([]);
  const [cliError, setCliError] = useState<string | null>(null);
  const [copiedCommand, setCopiedCommand] = useState<string | null>(null);
  useEffect(() => {
    getCliManifest()
      .then(setCliManifest)
      .catch(() => setCliError(t("settings.cli.loadError")));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const handleCopyExample = (doc: CliCommandDoc) => {
    navigator.clipboard
      .writeText(doc.example)
      .then(() => {
        setCopiedCommand(doc.name);
        setTimeout(() => setCopiedCommand((current) => (current === doc.name ? null : current)), 1500);
      })
      .catch(() => {});
  };

  // ── 정보(About) ──
  const [appVersion, setAppVersion] = useState<string | null>(null);
  useEffect(() => {
    getAppVersion()
      .then(setAppVersion)
      .catch(() => {});
  }, []);
  const handleOpenGithub = () => {
    void openExternalUrl("https://github.com/gradibo/bildorak").catch(() => {});
  };

  return (
    <div className="settings-view">
      <div className="page-eyebrow">{t("settings.eyebrow")}</div>
      <h1 className="page-title">{t("settings.title")}</h1>

      {saveError && (
        <div className="banner-error">
          <CheckStatusIcon status="fail" />
          <span>{saveError}</span>
        </div>
      )}

      <section className="settings-section">
        <h2 className="settings-section-title">{t("settings.flutterSdk.label")}</h2>
        <p className="settings-hint">{t("settings.flutterSdk.hint")}</p>
        <div className="settings-row settings-row-input">
          <input
            type="text"
            className="settings-text-input"
            value={flutterPathInput}
            onChange={(e) => setFlutterPathInput(e.target.value)}
            onBlur={() => void handleFlutterPathBlur()}
            placeholder="/opt/homebrew/bin/flutter"
            spellCheck={false}
            autoComplete="off"
          />
          <button type="button" className="btn btn-outline" disabled={detecting} onClick={() => void handleDetect()}>
            {detecting && <SpinnerIcon />}
            {detecting ? t("settings.flutterSdk.detecting") : t("settings.flutterSdk.detect")}
          </button>
        </div>
        {checking && <p className="settings-status">{t("settings.flutterSdk.checking")}</p>}
        {!checking && versionLine && <p className="settings-status settings-status-ok">{versionLine}</p>}
        {!checking && flutterError && (
          <div className="banner-error">
            <CheckStatusIcon status="fail" />
            <span>{flutterError}</span>
          </div>
        )}
      </section>

      <section className="settings-section">
        <h2 className="settings-section-title">{t("settings.language.label")}</h2>
        <div className="settings-row">
          <select
            className="settings-select"
            value={settings.language}
            onChange={(e) => void updateSettings({ language: e.target.value as Language })}
          >
            <option value="ko">한국어</option>
            <option value="en">English</option>
          </select>
        </div>
      </section>

      <section className="settings-section">
        <h2 className="settings-section-title">{t("settings.theme.label")}</h2>
        <div className="settings-row settings-segmented">
          {THEME_OPTIONS.map((option) => (
            <button
              key={option}
              type="button"
              className={`settings-segment${settings.theme === option ? " active" : ""}`}
              onClick={() => void updateSettings({ theme: option })}
            >
              {themeLabels[option]}
            </button>
          ))}
        </div>
      </section>

      <section className="settings-section">
        <div className="settings-row settings-row-toggle">
          <div>
            <h2 className="settings-section-title">{t("settings.notifications.label")}</h2>
            <p className="settings-hint">{t("settings.notifications.hint")}</p>
          </div>
          <label className="settings-toggle">
            <input
              type="checkbox"
              checked={settings.buildNotificationsEnabled}
              onChange={(e) => void updateSettings({ buildNotificationsEnabled: e.target.checked })}
            />
            <span className="settings-toggle-track" />
          </label>
        </div>
      </section>

      <section className="settings-section">
        <div className="settings-row settings-row-toggle">
          <div>
            <h2 className="settings-section-title">{t("settings.update.label")}</h2>
            <p className="settings-hint">{t("settings.update.hint")}</p>
          </div>
          <label className="settings-toggle">
            <input
              type="checkbox"
              checked={settings.autoUpdateCheckEnabled}
              onChange={(e) => void updateSettings({ autoUpdateCheckEnabled: e.target.checked })}
            />
            <span className="settings-toggle-track" />
          </label>
        </div>
      </section>

      <section className="settings-section">
        <h2 className="settings-section-title">{t("settings.vault.label")}</h2>
        {vaultError && (
          <div className="banner-error">
            <CheckStatusIcon status="fail" />
            <span>{vaultError}</span>
          </div>
        )}
        {vaultPath && <p className="settings-vault-path">{vaultPath}</p>}
        <div className="settings-row">
          <button type="button" className="btn btn-outline" disabled={!vaultPath} onClick={() => void handleOpenVault()}>
            {t("settings.vault.openFinder")}
          </button>
        </div>
      </section>

      <section className="settings-section">
        <h2 className="settings-section-title">{t("settings.cli.label")}</h2>
        <p className="settings-hint">{t("settings.cli.intro")}</p>
        {cliError && (
          <div className="banner-error">
            <CheckStatusIcon status="fail" />
            <span>{cliError}</span>
          </div>
        )}
        {cliManifest.length > 0 && (
          <div className="cli-command-list">
            {cliManifest.map((doc) => (
              <div className="cli-command-row" key={doc.name}>
                <code className="cli-command-name">
                  bildorak-cli {doc.name}
                  {doc.args ? ` ${doc.args}` : ""}
                </code>
                <p className="cli-command-desc">{doc.description}</p>
                <div className="cli-command-example">
                  <code>{doc.example}</code>
                  <button type="button" className="btn-text-secondary" onClick={() => handleCopyExample(doc)}>
                    {copiedCommand === doc.name ? t("settings.cli.copied") : t("settings.cli.copy")}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
        <p className="settings-hint settings-cli-binary-hint">{t("settings.cli.binaryHint")}</p>
      </section>

      <section className="settings-section">
        <h2 className="settings-section-title">{t("settings.about.label")}</h2>
        <p className="settings-about-line">
          {t("settings.about.version")} {appVersion ?? "…"}
        </p>
        <p className="settings-about-line">MIT License</p>
        <button type="button" className="btn-text-secondary" onClick={handleOpenGithub}>
          {t("settings.about.github")}
        </button>
      </section>
    </div>
  );
}
