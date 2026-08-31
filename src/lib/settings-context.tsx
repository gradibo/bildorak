// settings-context.tsx — 앱 설정(AppSettings) 전역 컨텍스트 + 가벼운 다국어(i18n) 제공. main.tsx 최상단
// 에서 한 번만 감싸고, 화면 어디서든 useSettings() 로 읽고/바꾼다(App.tsx 가 signingKeys 를 한 번
// 불러와 카드들에 내려주는 것과 같은 "단일 출처" 정신 — 다만 이건 여러 화면이 동시에 읽어야 해서
// props drilling 대신 context 를 쓴다).
//
// 언어 설정과 번역(t)이 같은 소스(settings.language)를 보므로 별도 컨텍스트로 쪼개지 않는다 —
// ⚠️ 번역 범위는 i18n.ts 사전에 있는 키뿐이다(설정 화면 자체 + 좌측 네비 + 일부 최상위 섹션 제목).
// 프로젝트 카드/점검 결과/에러 문구 등은 이번 범위 밖이라 여전히 한국어 고정이다.
//
// 테마(system/light/dark)는 <html data-theme="..."> 속성으로 적용한다 — 실제 색 값 전환은 App.css 의
// :root[data-theme="dark"] / :root:not([data-theme="light"]) 규칙이 담당한다(이 파일은 속성만 토글).
//
// 필드 변경마다 즉시 저장한다(별도 "저장" 버튼 없음) — 로컬 상태를 먼저 낙관적으로 갱신한 뒤 백엔드에
// 저장한다. 저장이 실패해도 되돌리지 않는다(설정 값은 재시도 비용이 낮다 — 실패하면 다음 변경 때 다시
// 시도되거나 재시작 후 이전 값으로 보일 뿐, 데이터 유실 위험이 없는 값들뿐이라 낙관적 갱신으로 충분하다는
// 판단). 실패는 호출부(SettingsView)가 잡아서 배너로 보여준다.

import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { getSettings, setSettings as persistSettings } from "./api";
import { DICTIONARY, type TranslationKey } from "./i18n";
import type { AppSettings, ThemePreference } from "./types";

const DEFAULT_SETTINGS: AppSettings = {
  flutterPath: null,
  language: "ko",
  theme: "system",
  buildNotificationsEnabled: true,
  autoUpdateCheckEnabled: true,
};

interface SettingsContextValue {
  settings: AppSettings;
  /** getSettings() 응답을 아직 못 받았으면 false — SettingsView 가 저장된 flutterPath 를 입력칸에
   * 채우는 시점을 여기 맞춘다(로드 전엔 settings 가 아직 DEFAULT_SETTINGS 라서, 로드 완료 시점을
   * 알아야 한다). */
  loaded: boolean;
  /** 바뀐 필드만 넘기면 나머지는 현재 값을 그대로 합쳐 저장한다. 실패하면 throw — 호출부가 잡아서
   * 보여준다(로컬 상태는 이미 낙관적으로 갱신된 뒤라 되돌리지 않는다, 파일 상단 문서 참고). */
  updateSettings: (patch: Partial<AppSettings>) => Promise<void>;
  t: (key: TranslationKey) => string;
}

const SettingsContext = createContext<SettingsContextValue | null>(null);

function applyTheme(theme: ThemePreference) {
  const root = document.documentElement;
  if (theme === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", theme);
  }
}

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [settings, setSettingsState] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    getSettings()
      .then((loadedSettings) => {
        setSettingsState(loadedSettings);
        applyTheme(loadedSettings.theme);
      })
      .catch(() => {
        // 못 불러와도 조용히 기본값(DEFAULT_SETTINGS, 이미 시스템 테마 적용된 상태)으로 계속 진행한다.
      })
      .finally(() => setLoaded(true));
  }, []);

  const updateSettings = async (patch: Partial<AppSettings>) => {
    const next = { ...settings, ...patch };
    setSettingsState(next);
    if (patch.theme) applyTheme(next.theme);
    await persistSettings(next);
  };

  const t = useMemo(() => {
    const dict = DICTIONARY[settings.language];
    return (key: TranslationKey) => dict[key] ?? DICTIONARY.ko[key] ?? key;
  }, [settings.language]);

  const value = useMemo(
    () => ({ settings, loaded, updateSettings, t }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [settings, loaded, t],
  );

  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}

export function useSettings(): SettingsContextValue {
  const ctx = useContext(SettingsContext);
  if (!ctx) {
    throw new Error("useSettings must be used within SettingsProvider");
  }
  return ctx;
}
