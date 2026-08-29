// types.ts — Rust 쪽 model.rs 와 1:1 대응하는 TS 타입. 필드명(camelCase)과 enum 값(소문자 문자열)은
// serde 직렬화 결과 그대로다(model.rs 의 #[serde(rename_all = ...)] 참고) — 여기서 변환하지 않는다.

export type Platform = "ios" | "android";

/** 점검 항목이 어느 OS 에서 의미가 있는지. "all" 이 아니면 카드에 OS 전용 표시를 붙인다. */
export type OsScope = "macos" | "windows" | "all";

export type CheckStatus = "pass" | "warn" | "fail";

export interface ProjectRecord {
  id: string;
  name: string;
  selectedPath: string;
  repoPath: string;
  version: string | null;
  buildNumber: string | null;
  platforms: Platform[];
  registeredAt: string;
}

export interface CheckItem {
  label: string;
  status: CheckStatus;
  message: string;
  nextAction?: string;
  os: OsScope;
}

export interface PreflightRun {
  id: string;
  projectId: string;
  startedAt: string;
  finishedAt: string;
  overallStatus: CheckStatus;
  checks: CheckItem[];
}

export const PLATFORM_LABEL: Record<Platform, string> = {
  ios: "iOS",
  android: "Android",
};

export const STATUS_LABEL: Record<CheckStatus, string> = {
  pass: "통과",
  warn: "주의",
  fail: "실패",
};

// ── 로컬 빌드 실행(2차) — model.rs 의 BuildTarget/BuildJobStatus/BuildJob/BuildStatus 와 1:1 대응. ──

/** 로컬에서 실행 가능한 빌드 대상 — 닫힌 집합(build.rs 의 resolve_command 고정 맵과 같은 값).
 * ios_release/android_release(release 빌드, 1차)도 디버그 타겟과 동일하게 게이트 없이 무료다
 * (2026-08-16 전 사용자 무료 전환) — 여기서는 값만 표현한다. */
export type BuildTarget = "ios_sim_debug" | "android_debug" | "ios_release" | "android_release";

/** 빌드 job 상태 — running/success/failed 3 가지만 쓴다(blocked 상태는 두지 않음). */
export type BuildJobStatus = "running" | "success" | "failed";

export interface BuildJob {
  id: string;
  projectId: string;
  target: BuildTarget;
  targetLabel: string;
  status: BuildJobStatus;
  startedAt: string;
  finishedAt: string | null;
  exitCode: number | null;
  pid: number | null;
  statusNote?: string;
}

/** get_build_status 커맨드 반환 형태 — job + 로그 tail + 산출물 확인 결과를 한 번에 담는다. */
export interface BuildStatus {
  job: BuildJob | null;
  logTail: string[];
  artifactRelpath?: string;
  artifactExists?: boolean;
}

/** 프로젝트가 감지한 플랫폼 → 그 플랫폼에서 실행 가능한 빌드 대상. ios_sim_debug 는 macOS 전용
 * 플래그다 — v0 는 macOS 만 지원하므로 여기서 따로 OS 분기하지 않는다. */
export const PLATFORM_BUILD_TARGET: Record<Platform, BuildTarget> = {
  ios: "ios_sim_debug",
  android: "android_debug",
};

/** 프로젝트가 감지한 플랫폼 → 그 플랫폼의 release 빌드 대상(1차, 무료 — PLATFORM_BUILD_TARGET 과
 * 같은 매핑 원칙, 디버그/릴리스를 별도 상수로 나눠 ProjectCard 가 두 줄로 나눠 보여줄 수 있게 한다). */
export const PLATFORM_RELEASE_BUILD_TARGET: Record<Platform, BuildTarget> = {
  ios: "ios_release",
  android: "android_release",
};

export const BUILD_TARGET_LABEL: Record<BuildTarget, string> = {
  ios_sim_debug: "iOS 시뮬레이터 디버그 빌드",
  android_debug: "Android 디버그 빌드",
  ios_release: "iOS 릴리스(ipa)",
  android_release: "Android 릴리스(aab)",
};

export const BUILD_STATUS_LABEL: Record<BuildJobStatus, string> = {
  running: "실행 중",
  success: "완료",
  failed: "실패",
};

// ── 서명키 관리(출시 준비 1차 골격) — model.rs 의 SigningKeyKind/SigningKeyRecord 와 1:1 대응. ──

/** 서명키 종류 — 닫힌 집합(model.rs::SigningKeyKind 와 같은 snake_case 값). */
export type SigningKeyKind = "ios_cert" | "ios_api_key" | "android_keystore";

/** 등록된 서명키 한 건 — list_signing_keys/register_signing_key/link_signing_key/unlink_signing_key
 * 커맨드가 그대로 반환한다. expiresAt 은 있으면 RFC3339 문자열(signing.rs::parse_enddate 가 openssl
 * 원문을 변환한 값) — 상태 판정은 직접 하지 말고 copy.ts::signingKeyExpiryStatus 를 통해서 한다
 * (kind == "ios_api_key" 는 expiresAt 이 없어도 "만료 없음"이라 단순 null 체크만으론 구분 안 됨). */
export interface SigningKeyRecord {
  id: string;
  kind: SigningKeyKind;
  displayName: string;
  filePath: string;
  expiresAt: string | null;
  linkedProjectIds: string[];
  /** kind === "android_keystore" 이고 register_android_signing 으로 비밀번호를 등록했을 때만 있다.
   * model.rs::AndroidSigningConfig 와 1:1 대응 — 비밀번호 원문은 여기 없다(keychain 참조만). */
  androidSigning?: AndroidSigningConfig;
  /** Android keystore 안전 보관 볼트 사본 경로(signing.rs::copy_keystore_into_vault, 확정된 설계 결정) —
   * kind === "android_keystore" 는 등록 시 항상 채워진다(이 기능 이전 레코드는 없음). filePath(원본,
   * 표시용)와 구분된다 — 실제 서명/검증은 이 경로를 쓴다(build.rs::resolve_android_signing). */
  vaultPath?: string;
}

export const SIGNING_KEY_KIND_LABEL: Record<SigningKeyKind, string> = {
  ios_cert: "iOS 인증서",
  ios_api_key: "App Store Connect API 키",
  android_keystore: "Android keystore",
};

/** Android release 서명 자동 주입 설정 — model.rs::AndroidSigningConfig 와 1:1 대응. 비밀번호 자체는
 * 여기 없다(macOS 키체인 서비스 이름 참조만) — 화면에는 "등록됨" 여부와 keyAlias 표시용으로만 쓴다. */
export interface AndroidSigningConfig {
  keyAlias: string;
  keychainAccount: string;
  storePasswordService: string;
  keyPasswordService: string;
  /** 인증서 만료일(RFC3339, 비밀 아님) — register_android_signing 등록 시점 keytool -list -v 스냅샷.
   * 등록 당시 keystore 를 못 찾았거나 keytool 파싱에 실패하면 null("확인 불가") — signingKeyExpiryStatus
   * 와는 별개 개념이다(이건 Android keystore 전용, 등록 시점 1회성 스냅샷이라 재계산되지 않는다). */
  certExpiry: string | null;
  /** 인증서 SHA-256 지문(콜론 구분 대문자 16진수, 비밀 아님) — certExpiry 와 같은 시점에 뽑은 값. */
  certSha256: string | null;
}

// ── 서명키/스토어 키 자동 탐색(다음 단계, keychain 이관 옵션 A) — model.rs 의 FoundKeyKind/FoundKey/
// P8Subtype/ImportAndroidSigningResult/FoundStoreKeyRecord 와 1:1 대응. ──

/** .p8 파일명 규칙으로만 구분한 세부 종류(key_scan.rs::parse_p8_filename) — AuthKey_*.p8 은 일반
 * App Store Connect API, SubscriptionKey_*.p8 은 인앱결제 구독 API 전용. */
export type P8Subtype = "app_store_connect_api" | "subscription" | "unknown";

export const P8_SUBTYPE_LABEL: Record<P8Subtype, string> = {
  app_store_connect_api: "App Store Connect API",
  subscription: "구독(인앱결제) API",
  unknown: "종류 확인 필요",
};

/** 스캔으로 찾은 키 한 건의 종류 — 태그 필드는 "type"(model.rs::FoundKeyKind 의 명시적 #[serde(rename)]
 * 값 그대로, 자동 케이스 변환에 기대지 않음). 비밀번호 "값"은 어느 variant 에도 없다 —
 * android_keystore 는 passwordsAvailable: boolean 하나로만 존재 여부를 알린다. */
export type FoundKeyKind =
  | {
      type: "android_keystore";
      alias: string | null;
      keyPropertiesPath: string | null;
      passwordsAvailable: boolean;
      /** 이 키가 쓰이는 안드로이드 앱의 applicationId(우선) 또는 namespace(폴백) — key_scan.rs::
       * find_app_id 가 build.gradle(.kts)에서 파싱한다(비밀번호 없이도 항상 시도). 근처에서 안드로이드
       * 프로젝트를 못 찾거나 파싱에 실패하면 null. */
      appId: string | null;
    }
  | {
      type: "apple_p8";
      keyId: string;
      subtype: P8Subtype;
    };

/** scan_signing_keys 커맨드가 반환하는 후보 키 한 건. */
export interface FoundKey {
  path: string;
  kind: FoundKeyKind;
  size: number;
  /** "YYYY-MM-DD"(UTC 기준 mtime 날짜). */
  modified: string;
  /** debug.keystore 는 애초에 scan_signing_keys 반환 목록에서 걸러진다(key_scan.rs) — 이 필드는 항상
   * false 로 온다고 봐도 되지만 스펙 형태를 그대로 유지한다. */
  isDebug: boolean;
}

/** import_found_android_signing 커맨드 반환 형태. */
export interface ImportAndroidSigningResult {
  key: SigningKeyRecord;
  /** true면 keychain 자동 이관 완료. false면 등록·연결만 하고 비밀번호는 못 찾음 — keyAlias 를 수동
   * 폼에 pre-fill 하고 비밀번호 입력을 받아야 한다(SigningKeysSection.tsx::FoundKeysPanel 참고). */
  imported: boolean;
  keyAlias: string | null;
}

/** register_found_store_key 로 저장한 ".p8 발견 기록" 한 건 — keychain 이관 없음(#6 스토어 업로드가
 * 나중에 사용). */
export interface FoundStoreKeyRecord {
  id: string;
  path: string;
  keyId: string;
  subtype: P8Subtype;
  registeredAt: string;
}

/** inspect_key_source 커맨드 반환 형태 — "등록" 클릭 직후, 실제 볼트 복사(최대 ~31초 재시도)를
 * 시도하기 전에 클라우드 온디맨드(다운로드 전) 상태인지 미리 알려준다(signing.rs::inspect_key_source,
 * stat 만 사용 — 비밀번호 등 비밀 값은 여기 없다). isCloud === false 면 isDownloaded 는 항상 true. */
export interface KeySourceInfo {
  isCloud: boolean;
  isDownloaded: boolean;
  folderName: string;
  /** isCloud 일 때만 의미 있다 — "Google Drive"/"iCloud"/"Dropbox"/"OneDrive" 중 하나 또는(이름 있는
   * 4가지 외 제공자) null. copy.ts::cloudKindLabelFromPath 가 null 일 때 화면 문구로 대체한다. */
  cloudKind: string | null;
}

// ── 설정(1차) — model.rs 의 AppSettings/Language/ThemePreference 와 1:1 대응. ──

export type Language = "ko" | "en";
export type ThemePreference = "system" | "light" | "dark";

export interface AppSettings {
  flutterPath: string | null;
  language: Language;
  theme: ThemePreference;
  buildNotificationsEnabled: boolean;
}

// ── CLI / 자동화(3단계, bildorak-cli) — model.rs 의 CliCommandDoc 과 1:1 대응. ──

/** CLI 서브커맨드 하나의 설명 — get_cli_manifest 커맨드가 배열로 반환한다(build.rs::cli_manifest() 가
 * 화면과 clap --help 양쪽의 단일 소스). name/args/example 은 사람이 터미널에 그대로 타이핑하는 형태의
 * 고정 문자열(실행에는 안 쓰인다). description/example 은 항상 한국어 원문 그대로 표시한다(이 화면의
 * i18n 범위는 섹션 제목/안내문 같은 구조적 라벨뿐 — i18n.ts 문서 참고). */
export interface CliCommandDoc {
  name: string;
  args: string;
  description: string;
  example: string;
}

/** 좌측 네비 화면 전환(App.tsx/Sidebar.tsx) — 등록된 앱 목록과 설정 화면 두 개뿐이다(v0). */
export type AppView = "projects" | "settings";
