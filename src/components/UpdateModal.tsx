// UpdateModal.tsx — 자동 업데이트(Tauri 공식 updater). 앱이 뜨면(설정 로드 완료 + "자동 업데이트
// 확인" 켜짐일 때만) GitHub Releases 의 latest.json 을 조용히 확인하고, 새 버전이 있을 때만 기존
// Modal 컴포넌트로 안내한다. 오프라인/릴리스 없음(404)/일시적 네트워크 오류는 전부 조용히 무시한다
// (콘솔에만 남김 — check() 는 이 세 경우 모두 reject 로 온다, tauri-plugin-updater::Updater::check
// 실측 확인: 404 는 Ok(None) 이 아니라 Err(ReleaseNotFound)). 사용자를 방해하지 않는 게 최우선이라
// catch 에서 그 이상 아무것도 하지 않는다.
//
// [지금 업데이트]는 downloadAndInstall 로 내려받기+설치를 한 번에 하고(진행 단계는 DownloadEvent 로
// 추적), 끝나면 relaunch() 로 새 버전을 재시작한다(macOS 는 설치만으론 안 바뀌고 재시작이 필요하다,
// @tauri-apps/plugin-updater 타입 주석 확인). [나중에]는 그냥 닫는다 — 다음 시작 때 다시 확인한다
// (별도 "무시함" 저장 없음, v0 범위).
import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { Modal } from "./Modal";
import { CheckStatusIcon, SpinnerIcon } from "./Icons";
import { useSettings } from "../lib/settings-context";

type InstallPhase = "idle" | "downloading" | "installing" | "relaunching" | "error";

export function UpdateModal() {
  const { settings, loaded, t } = useSettings();
  const [update, setUpdate] = useState<Update | null>(null);
  const [open, setOpen] = useState(false);
  const [phase, setPhase] = useState<InstallPhase>("idle");
  const [downloadPct, setDownloadPct] = useState<number | null>(null);

  // 설정이 로드되고("자동 업데이트 확인" 값을 알아야 하므로 loaded 를 기다린다, settings-context.tsx
  // 문서 참고) 그 값이 켜져 있을 때만 딱 한 번 확인한다. 언마운트/설정 변경 시 늦게 도착한 응답이 상태를
  // 건드리지 않도록 cancelled 플래그를 둔다(다른 화면의 useEffect 취소 패턴과 동일).
  useEffect(() => {
    if (!loaded || !settings.autoUpdateCheckEnabled) return;
    let cancelled = false;
    check()
      .then((found) => {
        if (cancelled || !found) return;
        setUpdate(found);
        setOpen(true);
      })
      .catch((e) => {
        console.debug("[bildorak] 업데이트 확인을 건너뜁니다(오프라인/릴리스 없음 등):", e);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, settings.autoUpdateCheckEnabled]);

  if (!update) return null;

  const busy = phase === "downloading" || phase === "installing" || phase === "relaunching";

  const handleClose = () => {
    if (busy) return; // 설치 진행 중엔 닫기(Escape/배경 클릭/X 전부 Modal.tsx 가 이 콜백 하나로 처리)를 막는다.
    setOpen(false);
  };

  const handleInstall = async () => {
    if (busy) return; // 버튼 disabled 에만 의존하지 않는 재진입 가드 - 이중 다운로드 방지.
    setPhase("downloading");
    setDownloadPct(null);
    let totalBytes = 0;
    let receivedBytes = 0;
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          totalBytes = event.data.contentLength ?? 0;
          receivedBytes = 0;
          setDownloadPct(totalBytes > 0 ? 0 : null);
        } else if (event.event === "Progress") {
          receivedBytes += event.data.chunkLength;
          setDownloadPct(totalBytes > 0 ? Math.min(100, Math.round((receivedBytes / totalBytes) * 100)) : null);
        } else if (event.event === "Finished") {
          setPhase("installing");
          setDownloadPct(null);
        }
      });
      setPhase("relaunching");
      await relaunch();
    } catch (e) {
      console.error("[bildorak] 업데이트 설치 실패:", e);
      setPhase("error");
    }
  };

  const phaseLabel: Record<"downloading" | "installing" | "relaunching", string> = {
    downloading: t("update.modal.downloading") + (downloadPct !== null ? ` ${downloadPct}%` : ""),
    installing: t("update.modal.installing"),
    relaunching: t("update.modal.relaunching"),
  };

  return (
    <Modal open={open} onClose={handleClose} title={t("update.modal.title").replace("{version}", update.version)}>
      <p className="update-release-notes">{update.body?.trim() || t("update.modal.noReleaseNotes")}</p>
      {phase === "error" && (
        <div className="banner-error">
          <CheckStatusIcon status="fail" />
          <span>{t("update.modal.error")}</span>
        </div>
      )}
      <div className="card-actions">
        <button type="button" className="btn btn-outline" disabled={busy} onClick={handleClose}>
          {t("update.modal.later")}
        </button>
        <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void handleInstall()}>
          {busy && <SpinnerIcon />}
          {busy ? phaseLabel[phase as "downloading" | "installing" | "relaunching"] : t("update.modal.installNow")}
        </button>
      </div>
    </Modal>
  );
}
