// api.ts — Rust 커맨드(src-tauri/src/commands.rs) 호출을 얇게 감싼다. 컴포넌트는 invoke() 를
// 직접 부르지 않고 이 함수들만 쓴다(엔진 원칙: 프론트는 project id/폴더 경로만 넘기고, 실행 명령은
// 전부 Rust 쪽 고정 로직이 결정한다 — 여기서도 그 경계를 그대로 유지).

import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  BuildJob,
  BuildStatus,
  BuildTarget,
  CliCommandDoc,
  FoundKey,
  FoundStoreKeyRecord,
  ImportAndroidSigningResult,
  KeySourceInfo,
  P8Subtype,
  PreflightRun,
  ProjectRecord,
  SigningKeyRecord,
} from "./types";

/** 네이티브 폴더 선택 창을 띄운다. 사용자가 취소하면 null. 반환값은 실제 경로가 아니라 Rust 쪽에
 * 보관된 선택 결과를 가리키는 1회용 토큰이다(경로 문자열 자체는 webview 로 넘어오지 않는다,
 * 설계 요구사항). */
export function pickProjectFolder(): Promise<string | null> {
  return invoke<string | null>("pick_project_folder");
}

/** 고른 폴더 토큰으로 pubspec.yaml 을 찾아 등록한다. 못 찾으면 비개발자 문구의 에러가 그대로 온다. */
export function registerProject(folderToken: string): Promise<ProjectRecord> {
  return invoke<ProjectRecord>("register_project", { folderToken });
}

export function listProjects(): Promise<ProjectRecord[]> {
  return invoke<ProjectRecord[]>("list_projects");
}

export function removeProject(projectId: string): Promise<void> {
  return invoke<void>("remove_project", { projectId });
}

export function runPreflight(projectId: string): Promise<PreflightRun> {
  return invoke<PreflightRun>("run_preflight", { projectId });
}

/** 로컬 빌드 실행(2차). target 은 enum 값만 넘긴다 — 실제 bin/args 는 Rust 쪽 고정 맵이 결정한다. */
export function startBuild(projectId: string, target: BuildTarget): Promise<BuildJob> {
  return invoke<BuildJob>("start_build", { projectId, target });
}

/** 현재/마지막 빌드 상태 — 앱 진입 시 복원, 진행 중일 때 폴링 양쪽에 쓴다. */
export function getBuildStatus(projectId: string): Promise<BuildStatus> {
  return invoke<BuildStatus>("get_build_status", { projectId });
}

/** 진행 중인 빌드를 취소한다(3차) — 무한 hang 시 앱 종료가 유일한 탈출구였던 상태를 해소한다. */
export function cancelBuild(projectId: string): Promise<void> {
  return invoke<void>("cancel_build", { projectId });
}

/** 빌드 히스토리 조회(2단계, 2026-08-16 전 사용자 무료 전환) — project_id 만 넘긴다. */
export function getBuildHistory(projectId: string): Promise<BuildJob[]> {
  return invoke<BuildJob[]>("get_build_history", { projectId });
}

// ── 서명키 관리(출시 준비 1차 골격) — 전부 register_project 와 같은 "표면 축소" 원칙을 따른다: 파일
// 경로 문자열은 webview 로 왕복하지 않고, 파일 선택 다이얼로그가 돌려준 1회용 토큰만 넘긴다
// (pickSigningKeyFile).

/** 네이티브 "파일 선택" 창을 띄운다(서명키 등록용). 사용자가 취소하면 null. */
export function pickSigningKeyFile(): Promise<string | null> {
  return invoke<string | null>("pick_signing_key_file");
}

export function listSigningKeys(): Promise<SigningKeyRecord[]> {
  return invoke<SigningKeyRecord[]>("list_signing_keys");
}

/** 고른 파일 토큰으로 종류를 감지하고 겉정보(만료일 등)를 추출해 등록한다. */
export function registerSigningKey(fileToken: string): Promise<SigningKeyRecord> {
  return invoke<SigningKeyRecord>("register_signing_key", { fileToken });
}

/** 등록 해제(완전 삭제) — 연결돼 있던 모든 프로젝트에서 함께 사라진다. 원본 키 파일은 건드리지 않는다. */
export function removeSigningKey(keyId: string): Promise<void> {
  return invoke<void>("remove_signing_key", { keyId });
}

/** 서명키를 프로젝트에 연결한다(다대다 — 하나의 인증서를 여러 앱에 쓸 수 있다). */
export function linkSigningKey(keyId: string, projectId: string): Promise<SigningKeyRecord> {
  return invoke<SigningKeyRecord>("link_signing_key", { keyId, projectId });
}

/** 이 프로젝트에서만 서명키 연결을 해제한다(레코드 자체는 남는다). */
export function unlinkSigningKey(keyId: string, projectId: string): Promise<SigningKeyRecord> {
  return invoke<SigningKeyRecord>("unlink_signing_key", { keyId, projectId });
}

/** Android release 자동 서명 비밀번호를 등록한다(다음 단계) — 비밀번호는 macOS 키체인에만 저장되고
 * webview 상태에는 이 호출 이후 남기지 않는다(SigningKeysSection 이 제출 직후 폼을 비운다). 등록해
 * 두면 release 빌드 시 자동으로 -P 서명 주입 + 빌드 후 서명 검증까지 한다(build.rs). */
export function registerAndroidSigning(
  keyId: string,
  keyAlias: string,
  storePassword: string,
  keyPassword: string,
): Promise<SigningKeyRecord> {
  return invoke<SigningKeyRecord>("register_android_signing", { keyId, keyAlias, storePassword, keyPassword });
}

/** 홑파일 keystore(등록 당시 옆에 key.properties 가 없는 경우)를 프로젝트에 등록·연결한 "다음" 자동으로
 * 시도하는 비밀번호 채움 — 그 프로젝트 자체의 key.properties(<repo_path>/android/key.properties)에서
 * 비밀번호를 찾아 storeFile 이 이 keystore 로 정확히 resolve 될 때만(안전 매칭) keychain 에 자동
 * 이관한다. 반환 형태는 importFoundAndroidSigning 과 같다(imported:false 면 프론트가 keyAlias 를 수동
 * 폼에 pre-fill 하고 비밀번호 입력을 받는다). */
export function autofillAndroidSigning(keyId: string, projectId: string): Promise<ImportAndroidSigningResult> {
  return invoke<ImportAndroidSigningResult>("autofill_android_signing", { keyId, projectId });
}

// ── 서명키/스토어 키 자동 탐색(다음 단계, keychain 이관 옵션 A) — 전부 register_signing_key 와 같은
// "표면 축소" 정신을 최대한 따르되, 스캔 결과 자체는 화면에 경로/메타데이터를 보여줘야 하는 기능이라
// FoundKey.path 는 예외적으로 webview 를 왕복한다(파일 선택 다이얼로그 토큰 패턴과 달리, 사용자가 직접
// 고른 파일이 아니라 자동으로 찾은 후보라 무엇을 찾았는지 보여줘야 선택할 수 있다). 비밀번호 "값"은
// 이 함수들의 반환값 어디에도 없다(commands.rs::scan_signing_keys/import_found_android_signing 주석).

/** 개발 머신의 고정 경로(스캔 규칙)를 스캔해 Android keystore/.p8 후보를 찾는다. 파일시스템을
 * 여러 곳 훑어 몇 초 걸릴 수 있다. */
export function scanSigningKeys(): Promise<FoundKey[]> {
  return invoke<FoundKey[]>("scan_signing_keys");
}

/** 스캔에서 찾은 Android keystore 를 프로젝트에 등록 + 연결한다. key.properties 에 비밀번호가 있으면
 * keychain 으로 자동 이관(imported: true), 없으면 등록·연결까지만 하고 imported: false 를 돌려준다 —
 * 프론트가 keyAlias 를 수동 입력 폼에 pre-fill 하고 비밀번호를 받는다. */
export function importFoundAndroidSigning(keystorePath: string, projectId: string): Promise<ImportAndroidSigningResult> {
  return invoke<ImportAndroidSigningResult>("import_found_android_signing", { keystorePath, projectId });
}

/** 발견한 .p8 스토어 키를 "발견 기록"만 저장한다(#6 스토어 업로드 기능이 나중에 사용, keychain 이관
 * 없음). 이미 기록된 경로면 새로 만들지 않고 기존 레코드를 그대로 돌려준다(멱등). */
export function registerFoundStoreKey(path: string, keyId: string, subtype: P8Subtype): Promise<FoundStoreKeyRecord> {
  return invoke<FoundStoreKeyRecord>("register_found_store_key", { path, keyId, subtype });
}

/** 이미 "발견 기록"된 .p8 목록 — 스캔 결과에서 이미 기록된 항목을 "기록됨"으로 표시하는 데 쓴다. */
export function listFoundStoreKeys(): Promise<FoundStoreKeyRecord[]> {
  return invoke<FoundStoreKeyRecord[]>("list_found_store_keys");
}

/** "등록" 클릭 직후, 실제 가져오기 전에 원본이 클라우드 온디맨드(다운로드 전) 상태인지 미리 확인한다
 * (stat 만 사용 — 다운로드를 유발하지 않는다). 재시도로 헛돌지 않고 즉시 안내하기 위한 사전 점검이라
 * project_id 는 필요 없다. */
export function inspectKeySource(path: string): Promise<KeySourceInfo> {
  return invoke<KeySourceInfo>("inspect_key_source", { path });
}

/** 클라우드 온디맨드 파일의 위치를 Finder 에서 강조 표시한다(reveal) — 파일을 열거나 다운로드를
 * 유발하지 않는다. inspectKeySource 가 "아직 다운로드되지 않았어요" 로 판정했을 때만 쓴다. */
export function revealSigningKeyInFinder(path: string): Promise<void> {
  return invoke<void>("reveal_signing_key_in_finder", { path });
}

/** 이 프로젝트의 Android applicationId(우선)/namespace(폴백) — 서명키 체크리스트 화면(SigningKeysSection)의
 * 앱 라벨에 쓴다. 못 찾으면(android 폴더가 없거나 build.gradle 파싱 실패) null — 화면은 앱 이름만 보여준다. */
export function getProjectAppId(projectId: string): Promise<string | null> {
  return invoke<string | null>("get_project_app_id", { projectId });
}

// ── 설정(1차) — model.rs::AppSettings 와 1:1 대응. commands.rs 의 설정 관련 커맨드만 부른다. ──

export function getSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("get_settings");
}

/** 설정 화면이 필드 하나가 바뀔 때마다 전체 스냅샷을 저장한다(부분 patch 아님, 별도 "저장" 버튼 없음). */
export function setSettings(settings: AppSettings): Promise<AppSettings> {
  return invoke<AppSettings>("set_settings", { newSettings: settings });
}

/** Flutter SDK 자동 감지("자동 감지" 버튼) — 못 찾으면 null(에러 아님). */
export function detectFlutterSdk(): Promise<string | null> {
  return invoke<string | null>("detect_flutter_sdk");
}

/** 주어진 경로가 실제 flutter 인지 확인하고 `flutter --version` 첫 줄을 돌려준다. */
export function checkFlutterPath(path: string): Promise<string> {
  return invoke<string>("check_flutter_path", { path });
}

/** 서명키 안전 보관 볼트 폴더 경로(표시용) — app_data_dir/keystores. */
export function getKeystoreVaultPath(): Promise<string> {
  return invoke<string>("get_keystore_vault_path");
}

/** 서명키 안전 보관 볼트 폴더를 Finder로 연다 — 경로는 항상 서버(Rust)가 결정한다(엔진 원칙). */
export function openKeystoreVault(): Promise<void> {
  return invoke<void>("open_keystore_vault");
}

/** 외부 링크(GitHub 저장소)를 시스템 기본 브라우저로 연다. https:// 로 시작하는 값만 허용된다(commands.rs). */
export function openExternalUrl(url: string): Promise<void> {
  return invoke<void>("open_external_url", { url });
}

/** 앱 버전(Cargo.toml/tauri.conf.json/package.json 이 항상 같은 값으로 유지된다). */
export function getAppVersion(): Promise<string> {
  return invoke<string>("get_app_version");
}

/** CLI 서브커맨드 문서 목록(3단계, bildorak-cli) — 설정 화면 "CLI / 자동화" 섹션에 쓴다. build.rs::
 * cli_manifest() 를 그대로 반환할 뿐이라 컴파일 타임 상수 목록이나 마찬가지다(외부 IO 없음). */
export function getCliManifest(): Promise<CliCommandDoc[]> {
  return invoke<CliCommandDoc[]>("get_cli_manifest");
}
