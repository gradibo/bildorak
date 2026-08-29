// copy.ts — 화면에 쓰는 순수 로직(IO 없음). nextRecommendedAction/formatKst/buildResultCopy 는
// 모두 같은 규칙을 따른다(비전공자 톤 통일).

import type { AndroidSigningConfig, BuildJob, BuildStatus, FoundKey, PreflightRun, SigningKeyRecord } from "./types";

/**
 * 카드에 보여줄 "다음 행동" 한 줄. 실행 이력이 없으면 점검부터 권한다. fail/warn 이 있으면 그
 * 항목의 nextAction(없으면 message)을 그대로, 전부 통과면 로컬 빌드 실행을 권한다(2차).
 */
export function nextRecommendedAction(run: PreflightRun | null): string {
  if (!run) return "빌드 준비 점검을 먼저 실행해 보세요.";
  const fail = run.checks.find((c) => c.status === "fail");
  if (fail) return fail.nextAction ?? fail.message;
  const warn = run.checks.find((c) => c.status === "warn");
  if (warn) return warn.nextAction ?? warn.message;
  return "모든 점검을 통과했어요. 아래에서 로컬 빌드를 실행할 수 있어요.";
}

/**
 * 빌드 실행 결과를 비전공자 톤 문장 2줄(headline/detail)로 변환 — 원본 로그(stdout/stderr)는 화면에서
 * "원본 로그 보기" 로 따로 펼쳐 보여준다(검증된 규칙).
 *
 * "빌드 중 앱을 닫으면 어떻게 되는지"는 실제 동작 그대로 서술한다 — bildorak 은 앱 종료 시 진행 중
 * 빌드를 process group 째로 정리한다(설계 원칙 — 좀비 프로세스 방지). 즉 앱을 닫으면 빌드도 실제로
 * 멈춘다 — 백그라운드에 남지 않는다. 문구가 실제 동작과 다르면 사용자가 오해하므로 그대로 반영한다.
 */
export function buildResultCopy(job: BuildJob): { headline: string; detail: string } {
  const target = job.targetLabel;
  switch (job.status) {
    case "running":
      return {
        headline: `${target}을 실행하고 있어요.`,
        detail: "빌드는 몇 분 정도 걸릴 수 있어요. 앱을 닫으면 빌드가 중단되니 끝날 때까지 켜 두세요.",
      };
    case "success":
      return {
        headline: "빌드가 완료되었습니다.",
        detail: "다음 단계: 아래 산출물 경로를 확인해 보세요.",
      };
    case "failed":
    default:
      return {
        headline: `${target} 중 문제가 발생했어요.`,
        detail: job.statusNote ?? "실패 원인: 빌드 중 오류가 발생했어요. 필요 행동: 원본 로그를 열어 원인을 확인하세요.",
      };
  }
}

/**
 * 빌드 산출물 확인 결과 한 줄 — success 가 아니면 null(아직 보여줄 게 없음). success 인데 실제
 * 파일이 없으면(artifactExists === false) "산출물 확인 필요" 로 안내한다(설계 스펙).
 */
export function artifactStatusLine(repoPath: string, status: BuildStatus): string | null {
  if (!status.job || status.job.status !== "success" || !status.artifactRelpath) return null;
  if (status.artifactExists) {
    return `산출물: ${repoPath}/${status.artifactRelpath}`;
  }
  return "산출물 확인 필요 — 예상 경로에서 결과물을 찾지 못했어요.";
}

/**
 * 점검 시각(ISO) → 한국 시간(KST) 한 줄. 로케일 의존 toLocaleString 대신 9시간 고정 가산
 * 방식을 쓴다(검증된 규칙).
 */
export function formatKst(at: string): string {
  const ms = Date.parse(at);
  if (Number.isNaN(ms)) return at;
  const kst = new Date(ms + 9 * 60 * 60 * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${kst.getUTCFullYear()}-${p(kst.getUTCMonth() + 1)}-${p(kst.getUTCDate())} ${p(kst.getUTCHours())}:${p(kst.getUTCMinutes())}`;
}

/**
 * 빌드 히스토리(2단계) 항목 하나의 소요시간을 "1분 20초" 같은 짧은 한국어 문구로. finishedAt 이
 * 없거나(이론상 히스토리엔 항상 있어야 하지만 방어적으로) 시각 파싱이 안 되면 빈 문자열(화면에는
 * 그냥 안 보여준다).
 */
export function buildDurationLabel(job: BuildJob): string {
  if (!job.finishedAt) return "";
  const startMs = Date.parse(job.startedAt);
  const endMs = Date.parse(job.finishedAt);
  if (Number.isNaN(startMs) || Number.isNaN(endMs) || endMs < startMs) return "";
  const totalSeconds = Math.round((endMs - startMs) / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return minutes === 0 ? `${seconds}초` : `${minutes}분 ${seconds}초`;
}

// ── 서명키 만료 상태(출시 준비 1차 골격) ──────────────────────────────────────
// preflight 의 CheckStatus(pass/warn/fail)는 Rust 쪽(model.rs::overall_status_of)이 판정해 내려주지만,
// 서명키 만료는 "지금 시각" 기준으로 매번 새로 계산해야 하는 값이라(등록 시점에 한 번 굳혀 버리면
// 화면을 열 때마다 다시 새로고침해야 최신 상태가 반영된다) 여기서 순수 함수로 매 렌더마다 계산한다.
// expiresAt 은 Rust 가 RFC3339 로 변환해 내려주므로(signing.rs::parse_enddate) Date.parse 가 항상
// 안전하다 — openssl 원문("Sep 15 05:14:35 2026 GMT" 같은 비표준 포맷)을 프론트에서 직접 파싱하지
// 않는다(엔진마다 파싱 결과가 다를 수 있어 위험, ECMA-262 는 ISO 8601 포맷만 파싱을 보장한다).

export type SigningKeyExpiryStatus = "valid" | "expiring_soon" | "expired" | "unknown" | "no_expiry";

/** 이 안쪽이면 "만료 임박"(노랑) — signing.rs 쪽 코멘트와 이 값을 맞춰 둔다(현재는 여기 한 곳뿐). */
const EXPIRY_WARNING_WINDOW_DAYS = 30;

/**
 * 서명키 만료 상태 신호등. kind == "ios_api_key" 는 원래 만료 개념이 없어 항상 no_expiry. 그 외
 * kind 에서 expiresAt 이 없거나 파싱이 안 되면 unknown("확인 불가") — Android keystore(1차 범위 밖),
 * .p12(암호 필요), openssl 파싱 실패/도구 없음이 전부 여기 해당한다. 하드 에러로 다루지 않는다
 * (preflight 의 "관대한 처리"와 같은 톤).
 */
export function signingKeyExpiryStatus(key: SigningKeyRecord): SigningKeyExpiryStatus {
  if (key.kind === "ios_api_key") return "no_expiry";
  if (!key.expiresAt) return "unknown";
  const ms = Date.parse(key.expiresAt);
  if (Number.isNaN(ms)) return "unknown";
  const daysLeft = (ms - Date.now()) / (1000 * 60 * 60 * 24);
  if (daysLeft < 0) return "expired";
  if (daysLeft <= EXPIRY_WARNING_WINDOW_DAYS) return "expiring_soon";
  return "valid";
}

/** 서명키 카드에 보여줄 짧은 상태 문구 — formatKst 로 날짜를 한국 시간 기준으로 붙인다. */
export function signingKeyExpiryLabel(key: SigningKeyRecord): string {
  const status = signingKeyExpiryStatus(key);
  switch (status) {
    case "no_expiry":
      return "만료 없음";
    case "unknown":
      return "만료 확인 불가";
    case "valid":
      return key.expiresAt ? `유효 (만료 ${formatKst(key.expiresAt)})` : "유효";
    case "expiring_soon":
      return key.expiresAt ? `만료 임박 (${formatKst(key.expiresAt)})` : "만료 임박";
    case "expired":
      return key.expiresAt ? `만료됨 (${formatKst(key.expiresAt)})` : "만료됨";
  }
}

// ── Android release 서명 인증서 겉정보(등록 시점 스냅샷) 표시 ──────────────────────────

/** SHA-256 지문을 앞 8바이트(그룹)만 남기고 줄인다 — 전체 32바이트를 화면에 다 보여줄 필요는 없다
 * (지문 비교는 keytool 원문으로 하지 이 축약 표시로 하지 않는다, 그냥 "등록됐다"는 확인용). 8그룹
 * 이하로 이미 짧으면(비정상 입력) 그대로 돌려준다. */
export function abbreviateSha256(sha256: string): string {
  const groups = sha256.split(":");
  if (groups.length <= 8) return sha256;
  return `${groups.slice(0, 8).join(":")}…`;
}

/**
 * Android release 서명 인증서의 만료일/지문 한 줄 — androidSigning 등록 시점 keytool 스냅샷(등록 이후
 * 다시 등록하기 전까지 갱신되지 않는다, model.rs::AndroidSigningConfig 문서 참고). 둘 다 없으면(등록
 * 당시 keystore 를 못 찾았거나 keytool 파싱 실패) null — "확인 불가" 배지를 따로 만들지 않고 화면에는
 * 그냥 안 보여준다(signingKeyExpiryStatus 의 unknown 과 겹치는 개념을 늘리지 않기 위해).
 */
export function androidCertMetaLine(config: AndroidSigningConfig): string | null {
  const parts: string[] = [];
  if (config.certExpiry) parts.push(`인증서 만료 ${formatKst(config.certExpiry)}`);
  if (config.certSha256) parts.push(`지문 ${abbreviateSha256(config.certSha256)}`);
  return parts.length > 0 ? parts.join(" · ") : null;
}

/**
 * Android keystore 안전 보관 상태 한 줄 — vaultPath 가 있으면(등록 시 항상 채워진다, 이 기능 이전
 * 레코드는 없음) "빌도락에 안전 보관됨 · 원본 <filePath>" 형태로 원본 위치를 함께 보여준다(원본이
 * 어디 있는지 알아볼 수 있게, 확정된 설계 결정 — 원본은 옮기지 않으니 filePath 가 항상 실제 원본 위치다).
 * 볼트 복사가 없으면(구버전 레코드) null — 화면은 그냥 안 보여준다.
 */
export function vaultStatusLine(key: SigningKeyRecord): string | null {
  if (!key.vaultPath) return null;
  return `빌도락에 안전 보관됨 · 원본 ${key.filePath}`;
}

// ── 자동 탐색(FoundKeysPanel) 표시 로직 ────────────────────────────────────────

/**
 * 스캔으로 찾은 Android keystore 의 "앱 추정명" — key.properties 에서 읽은 alias 가 있으면 그대로 쓰고,
 * 없으면 파일명(확장자 제외)으로 대신한다(signing.rs::build_record 의 display_name 이 파일명을 그대로
 * 쓰는 것과 같은 원칙). 실제 프로젝트 이름과 다를 수 있어 어디까지나 추정이다.
 */
export function foundAndroidKeyAppNameGuess(key: FoundKey): string {
  if (key.kind.type === "android_keystore" && key.kind.alias) return key.kind.alias;
  const fileName = key.path.split("/").pop() ?? key.path;
  const stem = fileName.replace(/\.(jks|keystore)$/i, "");
  return stem || "이름 확인 필요";
}

/**
 * 스캔으로 찾은 Android keystore 의 실제 앱 라벨 — key_scan.rs::find_app_id 가 build.gradle(.kts)에서
 * 읽은 applicationId/namespace 가 있을 때만 "→ com.example.myapp" 형태로 돌려준다. 못 찾았으면
 * (근처에 안드로이드 프로젝트가 없거나 파싱 실패) null — 화면은 foundAndroidKeyAppNameGuess(alias/파일명
 * 추정)만 그대로 보여준다(기존 폴백 유지).
 */
export function foundAndroidKeyAppIdLabel(key: FoundKey): string | null {
  if (key.kind.type !== "android_keystore" || !key.kind.appId) return null;
  return `→ ${key.kind.appId}`;
}

/**
 * repoPath 마지막 폴더 이름(예: "/Users/you/projects/myapp" → "myapp") — foundKeyMatchesProject 가
 * appId 매칭 외에 "폴더 이름이 파일명/경로에 포함되는지"로 후보를 넓힐 때 쓴다. 슬래시·백슬래시(윈도우
 * 경로) 둘 다 구분자로 보고, 끝에 구분자가 붙어 있어도(드묾) 마지막 세그먼트를 정확히 뽑는다.
 */
export function projectFolderName(repoPath: string): string {
  const segments = repoPath.split(/[/\\]/).filter((segment) => segment.length > 0);
  return segments.length > 0 ? segments[segments.length - 1] : "";
}

/** 폴더 이름이 이 값들이면(흔한 경로 토큰) foundKeyMatchesProject 의 "폴더 이름 포함" 매칭을 아예 하지
 * 않는다 — Android 프로젝트 구조상 거의 모든 keystore 경로에 "app"/"android" 같은 세그먼트가 등장해서,
 * projectFolder 가 우연히 이 값이 되면(예: repoPath 가 pubspec.yaml 위치상 ".../myapp/app" 처럼 앱
 * 하위 폴더까지 내려가 있어 마지막 세그먼트가 "app") 전혀 무관한 다른 프로젝트의 keystore 까지 전부
 * "이 앱 것 같아요"로 오매칭됐다(otherapp 키가 myapp 카드에 뜨는 실제 버그, 리뷰 지적). appId 정확
 * 일치 매칭(위)은 이 하드닝의 영향을 받지 않는다.
 */
const COMMON_PATH_TOKENS = new Set(["app", "android", "ios", "src", "lib", "main", "build", "java", "kotlin"]);

/**
 * 발견된 키가 "이 프로젝트 것 같은지" 판정 — FoundKeysPanel 배지("이 앱 것 같아요")·정렬에 쓴다. 기존
 * appId 일치(체크리스트가 이미 아는 recommendedAppId) 외에 매칭을 하나 더 둔다: 파일명/경로에 이
 * 프로젝트 폴더 이름이 포함되면(대소문자 무시) 힌트로 잡는다 — 홑파일 keystore(옆에 build.gradle 이
 * 없어 appId 를 못 구하는 myapp-upload-keystore.jks 같은 경우, 기존 appId 매칭만으론 전혀 안 걸렸다,
 * 리뷰 지적). 폴더 이름이 2자 이하면(예: "a", "ui") 거의 모든 경로에 우연히 들어맞아 오탐이 심하므로
 * 아예 비교하지 않는다 — 마찬가지 이유로 COMMON_PATH_TOKENS 에 있는 흔한 경로 단어도 비교하지 않는다
 * (2자 초과라도 "app"/"android" 등은 여전히 거의 모든 경로에 들어맞는다). Android keystore 후보에만
 * 적용한다(.p8 스토어 키는 체크리스트 "서명" 행과 무관해 대상이 아니다). 어디까지나 힌트일 뿐 — 자동
 * 등록은 하지 않고 사용자가 여전히 [등록]을 눌러야 한다.
 */
export function foundKeyMatchesProject(
  found: FoundKey,
  recommendedAppId: string | null,
  projectFolder: string,
): boolean {
  if (found.kind.type !== "android_keystore") return false;
  if (recommendedAppId && found.kind.appId === recommendedAppId) return true;
  const folder = projectFolder.toLowerCase();
  if (projectFolder.length > 2 && !COMMON_PATH_TOKENS.has(folder) && found.path.toLowerCase().includes(folder)) {
    return true;
  }
  return false;
}

// ── 클라우드 온디맨드 서명키 배지(경로 문자열 힌트, IO 없음) ──────────────────────────────────
// signing.rs::CLOUD_STORAGE_MARKERS/cloud_kind_label 과 같은 판정 기준을 프론트에도 둔다 — 스캔
// 목록의 카드마다 배지를 즉시 보여주려면(등록 전에 미리 알림) 항목마다 inspect_key_source 를 왕복
// 하는 대신 이미 갖고 있는 경로 문자열(FoundKey.path)만으로 먼저 힌트를 보여준다. 실제 등록 가능
// 여부(다운로드 여부까지)의 최종 판정은 여전히 "등록" 클릭 시점에 inspect_key_source(signing.rs,
// stat 실측)가 담당한다 — 이 배지는 사전 힌트일 뿐 그 판정을 대신하지 않는다(마커 목록이 두 곳에
// 있어 드물게 어긋나도, 실제 등록 흐름은 항상 Rust 쪽 실측을 따르므로 안전하다).
const CLOUD_PATH_MARKERS = ["/Library/CloudStorage/", "com~apple~CloudDocs", "Mobile Documents", "Dropbox", "OneDrive"];

export function looksLikeCloudPath(path: string): boolean {
  return CLOUD_PATH_MARKERS.some((marker) => path.includes(marker));
}

/** looksLikeCloudPath 가 true 인 경로의 제공자 이름 힌트 — signing.rs::cloud_kind_label 과 동일 기준
 * (이름 있는 4가지 라벨 우선). 그 외 CloudStorage 통합 제공자(예: Box)는 일반 문구로 물러난다 —
 * inspectKeySource 가 돌려주는 KeySourceInfo.cloudKind 도 이런 경우 null 이라 호출부가 `?? "클라우드
 * 저장소"` 로 같은 일반 문구를 쓴다(SigningKeysSection.tsx 참고). */
export function cloudKindLabelFromPath(path: string): string {
  if (path.includes("com~apple~CloudDocs") || path.includes("Mobile Documents")) return "iCloud";
  if (path.includes("GoogleDrive")) return "Google Drive";
  if (path.includes("Dropbox")) return "Dropbox";
  if (path.includes("OneDrive")) return "OneDrive";
  return "클라우드 저장소";
}
