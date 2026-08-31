// ProjectCard.tsx - 등록된 프로젝트 카드 하나.
// "빌드 준비 점검" 버튼 → Rust run_preflight 호출 → 통과/주의/실패 리스트 + "다음 행동" 문구.
// "로컬 빌드" 버튼(2차) → Rust start_build 호출 → 진행 중/성공/실패 상태 + 산출물 확인 + 원본 로그 보기.
// 톤은 디자인 확정 전 임시로 적용한 값이다(placeholder).

import { useEffect, useState } from "react";
import { cancelBuild, getBuildHistory, getBuildStatus, removeProject, runPreflight, startBuild } from "../lib/api";
import {
  artifactStatusLine,
  buildDurationLabel,
  buildResultCopy,
  formatKst,
  nextRecommendedAction,
} from "../lib/copy";
import {
  BUILD_STATUS_LABEL,
  BUILD_TARGET_LABEL,
  PLATFORM_BUILD_TARGET,
  PLATFORM_LABEL,
  PLATFORM_RELEASE_BUILD_TARGET,
  STATUS_LABEL,
  type BuildJob,
  type BuildJobStatus,
  type BuildStatus,
  type BuildTarget,
  type PreflightRun,
  type ProjectRecord,
  type SigningKeyRecord,
} from "../lib/types";
import { CheckStatusIcon, SpinnerIcon } from "./Icons";
import { ReleasesSection } from "./ReleasesSection";
import { SigningKeysSection } from "./SigningKeysSection";

const STATUS_PILL_CLASS: Record<PreflightRun["overallStatus"], string> = {
  pass: "pill-ok",
  warn: "pill-warn",
  fail: "pill-crit",
};

const BUILD_STATUS_PILL_CLASS: Record<BuildJobStatus, string> = {
  running: "pill-info",
  success: "pill-ok",
  failed: "pill-crit",
};

/** 진행 중일 때만 상태를 다시 물어보는 간격(ms) - 검증된 값 그대로 사용한다. */
const BUILD_POLL_INTERVAL_MS = 4000;

export function ProjectCard({
  project,
  onRemoved,
  signingKeys,
  projectNamesById,
  onSigningKeyRegistered,
  onSigningKeyRemoved,
  onSigningKeyUpdated,
}: {
  project: ProjectRecord;
  onRemoved: (id: string) => void;
  /** 전역 서명키 목록(App.tsx 가 한 번만 불러와 모든 카드에 그대로 내려준다) - SigningKeysSection 참고. */
  signingKeys: SigningKeyRecord[];
  projectNamesById: Record<string, string>;
  onSigningKeyRegistered: (key: SigningKeyRecord) => void;
  onSigningKeyRemoved: (keyId: string) => void;
  onSigningKeyUpdated: (key: SigningKeyRecord) => void;
}) {
  const [run, setRun] = useState<PreflightRun | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [removing, setRemoving] = useState(false);

  const [buildStatus, setBuildStatus] = useState<BuildStatus | null>(null);
  const [startingTarget, setStartingTarget] = useState<BuildTarget | null>(null);
  const [buildError, setBuildError] = useState<string | null>(null);
  const [buildLogOpen, setBuildLogOpen] = useState(false);
  const [cancelling, setCancelling] = useState(false);

  const [historyOpen, setHistoryOpen] = useState(false);
  const [history, setHistory] = useState<BuildJob[] | null>(null);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyError, setHistoryError] = useState<string | null>(null);

  const buildTargets = project.platforms.map((platform) => ({
    target: PLATFORM_BUILD_TARGET[platform],
    label: BUILD_TARGET_LABEL[PLATFORM_BUILD_TARGET[platform]],
  }));

  // release 빌드(1차) - buildTargets 와 같은 platforms 에서 나오므로 항상 같은 개수다. 디버그 타겟과
  // 동일하게 게이트 없이 무료다(2026-08-16 전 사용자 무료 전환).
  const releaseBuildTargets = project.platforms.map((platform) => ({
    target: PLATFORM_RELEASE_BUILD_TARGET[platform],
    label: BUILD_TARGET_LABEL[PLATFORM_RELEASE_BUILD_TARGET[platform]],
  }));

  // Android release 서명 자동 주입(다음 단계) - 이 프로젝트에 연결된 Android keystore 서명키 중
  // 비밀번호까지 등록된 게 있으면 release 빌드가 자동으로 서명 + 검증까지 한다(build.rs). iOS 는 아직
  // 이 범위 밖(프로젝트 자체 서명 설정을 그대로 따름)이라 문구를 플랫폼별로 나눈다.
  const androidSigningReady = signingKeys.some(
    (key) => key.kind === "android_keystore" && key.linkedProjectIds.includes(project.id) && key.androidSigning,
  );

  const versionLabel = project.version
    ? project.buildNumber
      ? `${project.version} (빌드 ${project.buildNumber})`
      : project.version
    : "확인 안 됨";

  const handleRunPreflight = async () => {
    if (running) return;
    setRunning(true);
    setError(null);
    try {
      const result = await runPreflight(project.id);
      setRun(result);
    } catch (e) {
      setError(typeof e === "string" ? e : "점검을 끝내지 못했어요. 잠시 후 다시 시도해 주세요.");
    } finally {
      setRunning(false);
    }
  };

  const handleRemove = async () => {
    if (removing) return;
    if (!window.confirm(`${project.name} 등록을 해제할까요? 프로젝트 폴더 자체는 지워지지 않아요.`)) {
      return;
    }
    setRemoving(true);
    try {
      await removeProject(project.id);
      onRemoved(project.id);
    } catch (e) {
      setError(typeof e === "string" ? e : "등록 해제를 완료하지 못했어요.");
      setRemoving(false);
    }
  };

  async function fetchBuildStatus() {
    try {
      const status = await getBuildStatus(project.id);
      setBuildStatus(status);
    } catch {
      // 상태 조회 실패는 조용히 무시 - 버튼 클릭이나 다음 폴링 때 다시 시도된다.
    }
  }

  // 최초 진입 시 한 번 - 이전 세션(앱 재시작 포함)에 시작된 빌드가 있으면 그 상태를 이어서 보여준다.
  useEffect(() => {
    if (buildTargets.length === 0) return;
    void fetchBuildStatus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.id]);

  // 진행 중일 때만 몇 초 간격으로 상태를 다시 물어본다 - 끝나면(성공/실패) 자동으로 멈춘다.
  useEffect(() => {
    if (buildStatus?.job?.status !== "running") return;
    const intervalId = setInterval(() => void fetchBuildStatus(), BUILD_POLL_INTERVAL_MS);
    return () => clearInterval(intervalId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.id, buildStatus?.job?.status]);

  // "빌드 기록" 섹션을 열 때마다 다시 불러온다 - 매번 새로 불러오므로 방금 끝난 빌드도 접었다 펼치면
  // 바로 반영된다.
  useEffect(() => {
    if (!historyOpen) return;
    setHistoryLoading(true);
    setHistoryError(null);
    getBuildHistory(project.id)
      .then(setHistory)
      .catch((e) => setHistoryError(typeof e === "string" ? e : "빌드 기록을 불러오지 못했어요."))
      .finally(() => setHistoryLoading(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [historyOpen, project.id]);

  const handleStartBuild = async (target: BuildTarget) => {
    if (startingTarget || buildStatus?.job?.status === "running") return;
    setStartingTarget(target);
    setBuildError(null);
    try {
      const job = await startBuild(project.id, target);
      setBuildStatus({ job, logTail: [] });
      setBuildLogOpen(false);
    } catch (e) {
      setBuildError(typeof e === "string" ? e : "빌드를 시작하지 못했어요. 잠시 후 다시 시도해 주세요.");
    } finally {
      setStartingTarget(null);
    }
  };

  // 무한 hang 시 앱 종료가 유일한 탈출구였던 상태 해소 - 진행 중인 빌드를 즉시 중단한다(설계
  // 요구사항). 취소 직후 바로 상태를 다시 물어봐 "취소됨" 결과를 화면에 곧장 반영한다.
  const handleCancelBuild = async () => {
    if (cancelling) return;
    setCancelling(true);
    setBuildError(null);
    try {
      await cancelBuild(project.id);
      await fetchBuildStatus();
    } catch (e) {
      setBuildError(typeof e === "string" ? e : "빌드를 취소하지 못했어요. 잠시 후 다시 시도해 주세요.");
    } finally {
      setCancelling(false);
    }
  };

  const buildJob = buildStatus?.job ?? null;
  const anyBuildRunning = buildJob?.status === "running";

  return (
    <div className="card">
      <div className="card-header">
        <div>
          <div className="card-title">{project.name}</div>
          <div className="card-platforms">
            {project.platforms.length === 0 ? (
              <span className="pill pill-idle">플랫폼 미확인</span>
            ) : (
              project.platforms.map((platform) => (
                <span key={platform} className="pill pill-info">
                  {PLATFORM_LABEL[platform]}
                </span>
              ))
            )}
          </div>
          <div className="card-path" title={project.repoPath}>
            {project.selectedPath}
          </div>
        </div>
        {run ? (
          <span className={`pill ${STATUS_PILL_CLASS[run.overallStatus]}`}>
            {STATUS_LABEL[run.overallStatus]}
          </span>
        ) : (
          <span className="pill pill-idle">미점검</span>
        )}
      </div>

      <div className="kv-row">
        <div>
          <div className="kv-label">버전</div>
          <div className="kv-value">{versionLabel}</div>
        </div>
        <div>
          <div className="kv-label">마지막 점검</div>
          <div className="kv-value" style={{ fontSize: 12 }}>
            {run ? formatKst(run.finishedAt) : "아직 안 함"}
          </div>
        </div>
      </div>

      <div className="next-action">다음 행동: {nextRecommendedAction(run)}</div>

      <div className="card-actions">
        <button type="button" className="btn btn-primary" disabled={running} onClick={() => void handleRunPreflight()}>
          {running && <SpinnerIcon />}
          {running ? "점검 중..." : "빌드 준비 점검"}
        </button>
      </div>

      {error && (
        <div className="banner-error">
          <CheckStatusIcon status="fail" />
          <span>{error}</span>
        </div>
      )}

      {run && (
        <ul className="check-list">
          {run.checks.map((check, index) => (
            <li key={`${check.label}-${index}`} className="check-item">
              <CheckStatusIcon status={check.status} />
              <div className="check-body">
                <div className="check-label-row">
                  <span className="check-label">{check.label}</span>
                  {check.os !== "all" && (
                    <span className="os-tag">{check.os === "macos" ? "macOS 전용" : "Windows 전용"}</span>
                  )}
                </div>
                <p className="check-message">{check.message}</p>
                {check.status !== "pass" && check.nextAction && (
                  <p className="check-next-action">필요한 행동: {check.nextAction}</p>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}

      <SigningKeysSection
        projectId={project.id}
        projectRootPath={project.selectedPath}
        allKeys={signingKeys}
        projectNamesById={projectNamesById}
        onRegistered={onSigningKeyRegistered}
        onRemoved={onSigningKeyRemoved}
        onUpdated={onSigningKeyUpdated}
      />

      <ReleasesSection
        projectId={project.id}
        projectVersionSnapshot={project.version}
        projectBuildNumberSnapshot={project.buildNumber}
      />

      {buildTargets.length > 0 && (
        <div className="build-section">
          <div className="build-section-label">로컬 빌드</div>
          <div className="card-actions">
            {buildTargets.map(({ target, label }) => {
              const isThisRunning = buildJob?.target === target && anyBuildRunning;
              const isStarting = startingTarget === target;
              return (
                <button
                  key={target}
                  type="button"
                  className="btn btn-outline"
                  disabled={isStarting || anyBuildRunning}
                  onClick={() => void handleStartBuild(target)}
                >
                  {(isStarting || isThisRunning) && <SpinnerIcon />}
                  {isThisRunning ? `${label} 실행 중...` : `${label} 실행`}
                </button>
              );
            })}
          </div>

          {releaseBuildTargets.length > 0 && (
            <>
              <div className="build-section-label">릴리스 빌드 · 스토어 업로드용</div>
              <p className="history-empty">
                {androidSigningReady
                  ? "Android는 등록한 keystore로 자동 서명하고, 빌드 후 서명이 맞는지도 자동으로 확인해요. iOS는 프로젝트 설정을 따라요."
                  : "Android는 서명키를 연결하고 비밀번호를 등록하면 자동 서명돼요(위 서명키 섹션). iOS는 프로젝트 설정을 따라요."}
              </p>
              <div className="card-actions">
                {releaseBuildTargets.map(({ target, label }) => {
                  const isThisRunning = buildJob?.target === target && anyBuildRunning;
                  const isStarting = startingTarget === target;
                  return (
                    <button
                      key={target}
                      type="button"
                      className="btn btn-outline"
                      disabled={isStarting || anyBuildRunning}
                      onClick={() => void handleStartBuild(target)}
                    >
                      {(isStarting || isThisRunning) && <SpinnerIcon />}
                      {isThisRunning ? `${label} 실행 중...` : `${label} 실행`}
                    </button>
                  );
                })}
              </div>
            </>
          )}

          {buildError && (
            <div className="banner-error">
              <CheckStatusIcon status="fail" />
              <span>{buildError}</span>
            </div>
          )}

          {buildJob && (
            <div className="build-status-box">
              <div className="build-status-row">
                <span className={`pill ${BUILD_STATUS_PILL_CLASS[buildJob.status]}`}>
                  {BUILD_STATUS_LABEL[buildJob.status]}
                </span>
                <span className="build-status-time">{formatKst(buildJob.finishedAt ?? buildJob.startedAt)}</span>
                {buildJob.status === "running" && (
                  <button
                    type="button"
                    className="btn-danger-text"
                    disabled={cancelling}
                    onClick={() => void handleCancelBuild()}
                  >
                    {cancelling ? "취소하는 중..." : "빌드 취소"}
                  </button>
                )}
              </div>
              {(() => {
                const copy = buildResultCopy(buildJob);
                return (
                  <>
                    <p className="build-headline">{copy.headline}</p>
                    <p className="build-detail">{copy.detail}</p>
                  </>
                );
              })()}
              {buildStatus && (() => {
                const artifactLine = artifactStatusLine(project.repoPath, buildStatus);
                return artifactLine ? <p className="build-artifact">{artifactLine}</p> : null;
              })()}
              <button
                type="button"
                className="build-log-toggle"
                onClick={() => setBuildLogOpen((open) => !open)}
              >
                {buildLogOpen ? "원본 로그 접기" : "원본 로그 보기"}
              </button>
              {buildLogOpen && (
                <pre className="build-log">
                  {buildStatus && buildStatus.logTail.length > 0
                    ? buildStatus.logTail.join("\n")
                    : "아직 로그가 없어요."}
                </pre>
              )}
            </div>
          )}

          <div className="history-section">
            <button type="button" className="history-toggle" onClick={() => setHistoryOpen((open) => !open)}>
              {historyOpen ? "빌드 기록 접기" : "빌드 기록 보기"}
            </button>
            {historyOpen && (
              <div className="history-list">
                {historyLoading && <p className="history-empty">불러오는 중...</p>}
                {historyError && (
                  <div className="banner-error">
                    <CheckStatusIcon status="fail" />
                    <span>{historyError}</span>
                  </div>
                )}
                {!historyLoading && !historyError && history && history.length === 0 && (
                  <p className="history-empty">아직 완료된 빌드 기록이 없어요.</p>
                )}
                {!historyLoading && history && history.length > 0 && (
                  <ul className="history-items">
                    {history.map((item) => (
                      <li key={item.id} className="history-item">
                        <span className={`pill ${BUILD_STATUS_PILL_CLASS[item.status]}`}>
                          {BUILD_STATUS_LABEL[item.status]}
                        </span>
                        <span className="history-item-target">{item.targetLabel}</span>
                        <span className="history-item-time">{formatKst(item.finishedAt ?? item.startedAt)}</span>
                        <span className="history-item-duration">{buildDurationLabel(item)}</span>
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            )}
          </div>
        </div>
      )}

      <div className="footer-note">
        <button type="button" className="btn-danger-text" disabled={removing} onClick={() => void handleRemove()}>
          {removing ? "해제하는 중..." : "등록 해제"}
        </button>
      </div>
    </div>
  );
}
