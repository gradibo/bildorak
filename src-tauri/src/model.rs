// model.rs - bildorak 공용 데이터 모델(JSON 직렬화 대상). IO 는 store.rs/preflight.rs/pubspec.rs 가 담당.
// 필드명은 camelCase 로 직렬화(프론트 TS 쪽 관례와 정렬 - repoPath/buildNumber/nextAction/
// overallStatus 등 화면에서 쓰는 이름 그대로 맞춘다).

use serde::{Deserialize, Serialize};

/// 감지된 플랫폼(ios/android).
/// v0 는 web 빌드는 범위 밖이라 web 은 없다(향후 로드맵 참고).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Ios,
    Android,
}

/// 점검 항목이 어느 OS 에서 의미가 있는지 - Windows 확장 시접(구현은 없음, 표시만).
/// "all" 이면 macOS/Windows 공통, "macos"/"windows" 면 해당 OS 전용 항목.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OsScope {
    Macos,
    Windows,
    All,
}

/// 등록된 프로젝트 한 건 - app config dir 의 projects.json 에 배열로 저장된다.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub id: String,
    /// pubspec.yaml 의 name 필드.
    pub name: String,
    /// 사용자가 실제로 고른 폴더(표시용) - pubspec 이 app/ 하위에서 발견됐으면 그 상위 폴더 그대로 보존.
    pub selected_path: String,
    /// pubspec.yaml 이 있는 실제 Flutter 프로젝트 루트. 모든 점검은 이 경로 하위로만 접근한다(엔진 원칙).
    pub repo_path: String,
    pub version: Option<String>,
    pub build_number: Option<String>,
    pub platforms: Vec<Platform>,
    pub registered_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

/// 점검 항목 하나 - 화면에 그대로 뿌려지는 형태(os 필드 포함).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckItem {
    pub label: String,
    pub status: CheckStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    pub os: OsScope,
}

/// 한 번의 "빌드 준비 점검" 실행 결과.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightRun {
    pub id: String,
    pub project_id: String,
    pub started_at: String,
    pub finished_at: String,
    pub overall_status: CheckStatus,
    pub checks: Vec<CheckItem>,
}

/// 점검 항목들의 전체 상태 - fail 하나라도 있으면 fail, 아니면 warn 있으면 warn, 아니면 pass.
/// (검증된 규칙 그대로 적용한다)
pub fn overall_status_of(checks: &[CheckItem]) -> CheckStatus {
    if checks.iter().any(|c| c.status == CheckStatus::Fail) {
        CheckStatus::Fail
    } else if checks.iter().any(|c| c.status == CheckStatus::Warn) {
        CheckStatus::Warn
    } else {
        CheckStatus::Pass
    }
}

// ── 로컬 빌드 실행 (2차) ─────────────────────────────────────────────────────
// 실제 실행 로직/IO 는 build.rs 가 담당(preflight.rs 가 이 파일의 CheckItem/PreflightRun 만 쓰고 IO 는
// 안 하는 것과 같은 경계 원칙). 여기 타입은 전부 build.rs + commands.rs + 프론트(types.ts)가 공유한다.

/// 로컬에서 실행 가능한 빌드 대상 - 닫힌 집합(고정). 실제 실행 바이너리/인자는 여기 없고
/// build.rs 의 resolve_command() 고정 맵에서만 나온다(확정된 설계 원칙).
/// IosRelease/AndroidRelease(release 빌드, 1차, 디버그 타겟과 동일하게 게이트 없이 무료 -
/// 2026-08-16 전 사용자 무료 전환, build.rs/model.rs 는 이 파일 전체 원칙대로 라이선스 개념을 모르는
/// 경계를 유지)는 스토어 업로드용 산출물(aab/ipa)을 만든다. **개인 키 서명 자체는 범위 밖**이다 -
/// Android 는 build.rs::resolve_android_signing 이 연결된 keystore 로 서명을 자동 주입하고, iOS 는
/// build.rs::resolve_ios_team_id 가 찾은 팀으로 app-store export 설정(ExportOptions.plist)만 만든다
/// (개인 키 서명 자체는 여전히 Xcode + keychain 인증서가 담당, 2026-08 추가). 서명 인증서/keystore 가
/// 아예 없는 프로젝트는 release 빌드가 실패할 수 있다(정상 - 에러 로그가 그대로 뜬다).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildTarget {
    IosSimDebug,
    AndroidDebug,
    IosRelease,
    AndroidRelease,
}

impl BuildTarget {
    /// 로그 파일명 등에 쓰는 고정 문자열 - serde 직렬화 결과(snake_case)와 항상 같다.
    pub fn as_str(self) -> &'static str {
        match self {
            BuildTarget::IosSimDebug => "ios_sim_debug",
            BuildTarget::AndroidDebug => "android_debug",
            BuildTarget::IosRelease => "ios_release",
            BuildTarget::AndroidRelease => "android_release",
        }
    }
}

/// 빌드 job 의 상태 - running/success/failed 3 가지만 쓴다(설계 스펙 그대로, blocked 상태는
/// 두지 않고 spawn 실패도 failed 로 합쳐 상태 종류를 늘리지 않는다).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildJobStatus {
    Running,
    Success,
    Failed,
}

/// 한 번의 로컬 빌드 실행 상태 - 프로젝트당 최신 1건만 저장한다(빌드 큐 없음).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildJob {
    pub id: String,
    pub project_id: String,
    pub target: BuildTarget,
    /// 시작 시점의 라벨 스냅샷 - 나중에 라벨 문구가 바뀌어도 과거 기록 표시가 흔들리지 않는다.
    pub target_label: String,
    pub status: BuildJobStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    /// 프로세스 종료 코드 - 아직 실행 중이거나 spawn 자체가 실패했으면 None.
    pub exit_code: Option<i32>,
    /// 생존 여부 실측(child_env::is_pid_alive)에 쓰는 pid - spawn 실패 시 None.
    pub pid: Option<u32>,
    /// spawn 실패 또는 stale 판정(running 인데 pid 가 죽어 강제 failed 처리)의 설명 문구.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_note: Option<String>,
}

/// get_build_status 커맨드 반환 형태 - job + 로그 tail + 산출물 확인 결과를 한 번에 묶는다
/// (프론트가 상태 조회 한 번으로 카드 전체를 그릴 수 있게 하나로 묶은 모양).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildStatus {
    pub job: Option<BuildJob>,
    pub log_tail: Vec<String>,
    /// job 이 있을 때만 채운다(target 에 따른 고정 상대경로, repoPath 기준).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_relpath: Option<String>,
    /// success 여부와 무관하게 "지금 그 경로가 실제로 있는지" 실측값 - success 인데 false 면 프론트가
    /// "산출물 확인 필요" 문구를 보여준다(설계 스펙).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_exists: Option<bool>,
}

// ── 서명키 관리(출시 준비 1차 골격) ───────────────────────────────────────────
// 실제 IO(등록 목록 읽기/쓰기, 파일 종류 감지, openssl 로 만료일 추출)는 signing.rs 가 담당한다
// (model.rs 는 데이터 모양만 - 파일 상단 주석 원칙 그대로). 커맨드는 commands.rs 의
// list_signing_keys/register_signing_key/remove_signing_key/link_signing_key/unlink_signing_key.
// 서명키 관리는 게이트 없이 무료다(get_build_history 와 같은 원칙 - model.rs 는 라이선스 개념 자체를
// 모르는 경계를 유지). 실제 배포 서명·스토어 업로드는 이번 범위 밖(다음 로드맵 단계).

/// 서명키 종류 - 닫힌 집합, 파일 확장자로 감지한다(signing.rs::detect_kind). p12(암호로 보호된
/// 인증서, 흔히 Keychain 에서 내보낸 iOS 배포/개발 인증서 형식)도 cer/pem/crt 와 같은 IosCert 로
/// 묶는다 - 1차 골격에서는 종류만 감지하고 만료 파싱은 미룬다(암호가 필요해서, 아래
/// SigningKeyRecord::expires_at 주석 참고).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SigningKeyKind {
    IosCert,
    IosApiKey,
    AndroidKeystore,
}

/// 등록된 서명키 한 건 - app config dir 의 signing_keys.json 에 배열로 저장된다(signing.rs).
///
/// ⚠️ 보안: 이 구조체 어디에도 키의 비밀 내용(private key 바이트·비밀번호·인증서 payload)을 담지
/// 않는다. filePath 는 사용자가 원본을 둔 위치를 가리키기만 할 뿐 파일 자체를 복사하지 않고, 여기
/// 저장되는 값은 전부 "겉정보"(종류·이름·만료일)뿐이다(signing.rs 파일 상단 주석 참고).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SigningKeyRecord {
    pub id: String,
    pub kind: SigningKeyKind,
    /// 화면에 보여줄 이름 - 1차는 파일명 그대로(signing.rs::build_record).
    pub display_name: String,
    /// 사용자가 원본 파일을 둔 위치 그대로(표시용) - kind == AndroidKeystore 는 등록 시 이 경로의
    /// 사본을 vault_path 로 별도 보관하지만 file_path 자체는 항상 원본 위치를 가리킨다(화면에 "원본
    /// 위치"로 보여줄 값, 이동·삭제·수정 절대 금지). iOS 인증서/API 키는 여전히 원본을 복사하지 않아
    /// file_path 가 유일한 실사용 경로다.
    pub file_path: String,
    /// RFC3339 문자열(다른 타임스탬프 필드와 동일 규칙) - signing.rs::parse_enddate 가 openssl 원문을
    /// 변환한다. kind == IosApiKey 는 원래 만료 개념이 없어 항상 None("만료 없음"으로 표시). 그 외
    /// kind 에서 None 이면 파싱 실패/암호 필요(.p12)/도구 없음/Android keystore(1차 범위 밖) 등
    /// "확인 불가"를 뜻한다 - 프론트(copy.ts::signingKeyExpiryStatus)가 kind 와 함께 봐서 두 의미를
    /// 구분한다.
    pub expires_at: Option<String>,
    /// 이 서명키를 쓰는 프로젝트 id 목록 - 다대다 관계(하나의 인증서를 여러 앱에 쓸 수 있다).
    pub linked_project_ids: Vec<String>,
    /// Android release 서명 자동 주입 설정(다음 단계) - kind ==
    /// AndroidKeystore 인 레코드가 signing.rs::register_android_signing 으로 keychain 에 비밀번호를
    /// 저장한 "다음"에만 채워진다. ⚠️ 비밀번호 자체는 여기 없다 - keychain 서비스 이름(참조)만 저장한다
    /// (이 struct 상단 보안 주석과 동일 원칙). 등록 전이거나 다른 kind 는 항상 None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub android_signing: Option<AndroidSigningConfig>,
    /// Android keystore 안전 보관 볼트 사본의 실제 경로(signing.rs::copy_keystore_into_vault 가 앱
    /// 데이터 폴더 하위 keystores/ 에 복사해 둔 위치, 확정된 설계 결정 - keystore 분실 대비 백업) -
    /// kind == AndroidKeystore 는 등록 시점에 항상 채워진다(register_signing_key/key_scan.rs::
    /// import_android_signing 둘 다 저장 전에 이 필드를 채운다). 이후 실제 서명(build.rs::
    /// resolve_android_signing)과 인증서 겉정보 재조회(signing.rs::register_android_signing 호출부)는
    /// 이 경로를 우선 쓴다(원본이 옮겨지거나 사라져도 자체 완결) - file_path(원본)는 화면에 "원본
    /// 위치"를 보여주는 표시용으로만 남는다. 이 기능 이전에 등록된 레코드나 iOS 종류는 None(하위 호환 -
    /// 실사용 코드는 없으면 file_path 로 폴백한다).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
}

/// Android keystore 의 release 서명 자동 주입에 필요한 겉정보 - 비밀번호는 여기 없고 macOS 키체인
/// generic-password 항목을 가리키는 서비스 이름만 있다(signing.rs::store_keychain_password /
/// read_keychain_password 가 이 서비스 이름 + keychain_account 로 실제 비밀번호를 읽고 쓴다). store
/// 비밀번호와 key 비밀번호는 값이 다를 수 있어(전통 JKS 포맷은 서로 다른 값을 지원 - 실측 확인,
/// keytool -genkeypair -storetype JKS) keychain 항목도 서비스 이름을 store/key 로 나눠 따로 저장한다.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidSigningConfig {
    /// keystore 안에서 서명에 쓸 key alias - flutter -P android.injected.signing.key.alias 로 그대로
    /// 전달된다.
    pub key_alias: String,
    /// keychain 조회에 쓰는 account(-a 값) - 항상 key_alias 와 같은 값(등록 시 signing.rs 가 그렇게
    /// 채운다), 두 값을 분리해 둔 건 keychain 조회 코드가 "이 레코드가 어떤 account 로 저장했는지"를
    /// key_alias 와 별개로 명시적으로 갖게 하기 위함(향후 alias 와 account 규칙이 갈라져도 이 구조체만
    /// 보면 된다).
    pub keychain_account: String,
    /// store 비밀번호가 저장된 keychain 서비스 이름(-s 값, "bildorak.signing.<keyId>.store").
    pub store_password_service: String,
    /// key 비밀번호가 저장된 keychain 서비스 이름(-s 값, "bildorak.signing.<keyId>.key").
    pub key_password_service: String,
    /// 인증서 만료일(RFC3339, **비밀 아님**) - signing.rs::register_android_signing 이 등록 시점에
    /// keytool -list -v 로 한 번 뽑은 스냅샷이다(SigningKeyRecord::expires_at 과 동일하게 프론트가
    /// Date.parse 로 바로 쓸 수 있는 포맷). keystore 파일을 못 찾았거나 비밀번호/별칭이 안 맞거나 keytool
    /// 파싱에 실패해도 등록 자체는 계속 진행하고 이 값만 None("확인 불가")이 된다 - 하드 에러 아님. 이후
    /// alias/비밀번호를 다시 등록하기 전까지는 갱신되지 않는다.
    pub cert_expiry: Option<String>,
    /// 인증서 SHA-256 지문(콜론 구분 대문자 16진수, **비밀 아님**) - 위 cert_expiry 와 같은 시점에 같은
    /// keytool 출력에서 뽑는다(signing.rs::extract_sha256_fingerprint, build.rs::verify_release_signing 이
    /// 빌드 후 비교하는 값과 동일 포맷). 실패 규칙도 cert_expiry 와 동일(None = 확인 불가).
    pub cert_sha256: Option<String>,
}

// ── 서명키/스토어 키 자동 탐색(다음 단계, keychain 이관 옵션 A) ─────────────────────────────
// 실제 파일시스템 스캔 + key.properties 파싱은 key_scan.rs 가 담당한다(model.rs 는 데이터 모양만 -
// 파일 상단 원칙 그대로). 커맨드는 commands.rs 의 scan_signing_keys/import_found_android_signing/
// register_found_store_key/list_found_store_keys.
//
// ⚠️ 보안: 아래 어떤 타입에도 keystore 비밀번호 "값"이 없다 - FoundKeyKind::AndroidKeystore 는
// passwordsAvailable: bool 하나로만 존재 여부를 알린다. key_scan.rs::KeyPropertiesFound(비밀번호 원문을
// 잠깐 들고 있는 내부 전용 타입)는 Serialize 를 derive 하지 않아 애초에 어떤 커맨드 반환값에도 실릴 수
// 없다 - 이 파일의 타입들과는 구조적으로 분리되어 있다.

/// App Store Connect API 키(.p8) 세부 종류 - 파일명 규칙으로만 구분한다(key_scan.rs::parse_p8_filename).
/// AuthKey_*.p8 은 일반 App Store Connect API(앱 정보/빌드 업로드), SubscriptionKey_*.p8 은 인앱결제
/// 구독 API 전용 - Apple 개발자 포털이 실제로 이 접두사로 구분해서 내려준다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum P8Subtype {
    AppStoreConnectApi,
    Subscription,
    Unknown,
}

/// 스캔으로 찾은 키 한 건의 종류 + 겉정보. SigningKeyKind(단순 문자열)와 달리 종류마다 필요한 부가
/// 필드가 달라 내부 태그 방식("type" 필드)으로 직렬화한다.
/// ⚠️ 태그 문자열은 자동 케이스 변환(rename_all)에 기대지 않고 각 variant 에 명시로 고정한다 - "P8"
/// 처럼 숫자가 섞인 variant 이름은 자동 변환 결과가 사람 눈에 자명하지 않아서다(model.rs 아래 tests
/// 모듈이 실제 직렬화 문자열을 실측 고정해 둔다).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FoundKeyKind {
    // ⚠️ rename_all 은 enum 컨테이너에 붙이면 "variant 이름"(태그 값)에만 적용되고 struct-variant 의
    // 필드에는 적용되지 않는다(rename = "..." 로 이미 태그 값을 명시했으니 무의미) - 필드를 camelCase 로
    // 내보내려면 이렇게 각 variant 에 rename_all 을 따로 붙여야 한다(model.rs 아래 tests 모듈이 실측으로
    // 잡아낸 문제 - 처음엔 enum 컨테이너에만 붙였다가 storePassword 류 필드가 snake_case 로 나가는 걸
    // 테스트가 잡았다).
    #[serde(rename = "android_keystore", rename_all = "camelCase")]
    AndroidKeystore {
        /// key.properties 에서 읽은 keyAlias(있으면). 없으면 프론트가 파일명으로 대신 표시한다
        /// (copy.ts::foundAndroidKeyAppNameGuess).
        alias: Option<String>,
        /// 실제로 사용한 key.properties 파일 경로(있으면) - 비밀 아님, 표시용.
        key_properties_path: Option<String>,
        /// storePassword/keyPassword 가 둘 다 있는지 여부만 - 값 자체는 여기 없다(파일 상단 보안 주석).
        passwords_available: bool,
        /// 이 키가 쓰이는 안드로이드 앱의 applicationId(우선) 또는 namespace(폴백) - key_scan.rs::
        /// find_app_id 가 key.properties/keystore 위치 근처의 build.gradle(.kts)에서 파싱한다(비밀번호
        /// 없이도 항상 시도 - build.gradle 은 비밀이 아니다). 근처에서 안드로이드 프로젝트를 못 찾거나
        /// 파싱에 실패하면 None - 프론트는 그때 기존 alias/파일명 추정(foundAndroidKeyAppNameGuess)만
        /// 보여준다.
        app_id: Option<String>,
    },
    #[serde(rename = "apple_p8", rename_all = "camelCase")]
    AppleP8 {
        /// 파일명(AuthKey_<KEYID>.p8 / SubscriptionKey_<KEYID>.p8)에서 뽑은 Key ID.
        key_id: String,
        subtype: P8Subtype,
    },
}

/// scan_signing_keys 커맨드가 돌려주는 후보 키 한 건.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundKey {
    pub path: String,
    pub kind: FoundKeyKind,
    pub size: u64,
    /// "YYYY-MM-DD"(UTC 기준 mtime 날짜) - key_scan.rs::format_modified.
    pub modified: String,
    /// debug.keystore(자동 생성되는 디버그 전용 키)면 true. ⚠️ 이 값이 true 인 항목은 애초에
    /// scan_signing_keys 반환 목록에 포함되지 않는다(수집 단계에서 걸러짐, key_scan.rs::
    /// collect_if_candidate 참고) - 필드 자체는 분류 로직을 독립적으로 테스트할 수 있게, 그리고 스펙에
    /// 명시된 형태를 그대로 유지하기 위해 남겨 둔다.
    pub is_debug: bool,
}

/// import_found_android_signing 커맨드 반환 형태.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAndroidSigningResult {
    /// 등록(+이 프로젝트에 연결)된 서명키 레코드 - imported == true 면 androidSigning 도 채워져 있다.
    pub key: SigningKeyRecord,
    /// true 면 key.properties 의 비밀번호를 keychain 으로 자동 이관까지 완료. false 면 등록·연결까지만
    /// 하고 비밀번호는 못 찾았다 - 프론트가 keyAlias 를 수동 폼에 pre-fill 하고 비밀번호 입력을 받는다.
    pub imported: bool,
    /// key.properties 에서 찾은 keyAlias(있으면) - imported == false 여도 수동 폼 pre-fill 용으로
    /// 내려준다.
    pub key_alias: Option<String>,
}

/// register_found_store_key 로 저장하는 ".p8 발견 기록" 한 건 - key_scan.rs 의 store_keys.json 에
/// 배열로 저장된다(signing_keys.json 과 별개 파일 - .p8 은 아직 keychain 이관을 하지 않는 가벼운
/// 기록이라 관심사를 분리했다). 로드맵 #6(스토어 자동 업로드)이 나중에 이 기록을
/// 읽어 쓴다.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundStoreKeyRecord {
    pub id: String,
    pub path: String,
    pub key_id: String,
    pub subtype: P8Subtype,
    pub registered_at: String,
}

// ── 클라우드 키 선제 알림(등록 전 사전 확인) ────────────────────────────────────────────────
// 실제 판정(경로 마커 매칭 + stat 실측)은 signing.rs::inspect_key_source 가 담당한다(model.rs 는
// 데이터 모양만 - 파일 상단 원칙 그대로). 커맨드는 commands.rs::inspect_key_source.

/// inspect_key_source 커맨드가 돌려주는 등록 전 사전 확인 결과 - "등록" 버튼을 누른 직후, 실제
/// 볼트 복사(signing.rs::copy_keystore_into_vault, 최대 ~31초 재시도)를 시도하기 전에 클라우드
/// 온디맨드(다운로드 전) 상태인지 먼저 알려준다(리뷰 지적 - 재시도로 헛돌지 않고 즉시 안내).
/// ⚠️ 비밀번호 등 비밀 값은 이 구조체 어디에도 없다 - 파일 경로 기반 겉정보(클라우드 여부·다운로드
/// 여부·폴더 이름)뿐이다.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeySourceInfo {
    /// signing.rs::looks_like_cloud_path 판정 그대로 - 클라우드 온디맨드 폴더(iCloud/Google Drive/
    /// Dropbox/OneDrive) 경로 마커가 있으면 true.
    pub is_cloud: bool,
    /// stat(2) 만으로 판정(signing.rs::is_file_downloaded) - 파일을 열거나 다운로드를 유발하지
    /// 않는다. is_cloud == false 면 항상 true(로컬 파일은 온디맨드 개념이 없다).
    pub is_downloaded: bool,
    /// 이 파일의 부모 폴더 이름(표시용, 예: "하루블록키") - 못 구하면(경로에 부모가 없는 극단적
    /// 경우) 빈 문자열.
    pub folder_name: String,
    /// is_cloud == true 일 때만 의미 있다 - "Google Drive"/"iCloud"/"Dropbox"/"OneDrive" 중 경로
    /// 마커로 식별된 이름. 그 외 CloudStorage 통합 제공자(예: Box)는 이름을 추측하지 않고 None -
    /// 프론트가 일반 문구("클라우드 저장소")로 대신 표시한다(copy.ts::cloudKindLabelFromPath).
    pub cloud_kind: Option<String>,
}

// ── 앱 설정(1차, 설정 화면) ───────────────────────────────────────────────────
// 실제 IO(읽기/쓰기, Flutter SDK 자동 감지/검증)는 settings.rs 가 담당한다(model.rs 는 데이터 모양만 -
// 파일 상단 원칙 그대로). 커맨드는 commands.rs 의 get_settings/set_settings/detect_flutter_sdk/
// check_flutter_path/get_keystore_vault_path/open_keystore_vault/open_external_url/get_app_version.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Ko,
    En,
}

fn default_language() -> Language {
    Language::Ko
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

fn default_theme() -> ThemePreference {
    ThemePreference::System
}

fn default_true() -> bool {
    true
}

/// 앱 설정 - app config dir 의 settings.json 에 저장된다(settings.rs). 다른 *.json(projects.json 등)과
/// 달리 배열이 아니라 단일 객체다(설정은 늘 정확히 하나). 필드 전부 #[serde(default...)] 를 둬서
/// 파일이 없거나(첫 실행) 이 기능 이전 버전의 파일(필드 일부만 있음)이어도 항상 유효한 값을 만든다.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Flutter SDK 실행 파일 절대경로 - 없으면(None) build.rs 가 기존처럼 PATH 의 "flutter"를 그대로
    /// 쓴다(무회귀, build.rs::start_build / settings.rs::resolve_flutter_bin 참고).
    #[serde(default)]
    pub flutter_path: Option<String>,
    #[serde(default = "default_language")]
    pub language: Language,
    #[serde(default = "default_theme")]
    pub theme: ThemePreference,
    /// 빌드 완료 macOS 알림(commands.rs::spawn_build_finish_notifier) 표시 여부 - 기본 켬(이 설정
    /// 도입 이전 동작 그대로, 무회귀).
    #[serde(default = "default_true")]
    pub build_notifications_enabled: bool,
    /// 앱 시작 시 GitHub Releases 새 버전을 조용히 확인할지(자동 업데이트, UpdateModal.tsx) - 기본 켬.
    /// 꺼도 설정 자체는 남아 있고 프론트가 시작 시 check() 호출을 건너뛸 뿐이다(Rust 쪽은 게이트하지
    /// 않는다 - build_notifications_enabled 와 달리 이 값을 읽는 커맨드가 없다).
    #[serde(default = "default_true")]
    pub auto_update_check_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            flutter_path: None,
            language: default_language(),
            theme: default_theme(),
            build_notifications_enabled: true,
            auto_update_check_enabled: true,
        }
    }
}

// ── 릴리스 관리(1차 슬라이스) ──────────────────────────────────────────────────
// 실제 IO(등록 목록 읽기/쓰기)는 releases.rs 가 담당한다(model.rs 는 이 파일 전체 원칙대로 데이터
// 모양만 정의). 커맨드는 commands.rs 의 list_releases/create_release/update_release/delete_release/
// get_project_current_version. 빌드 이력 연결·GitHub 연동·제출 자동화·다중 스토어 상태·구조화 노트는
// 이번 범위 밖(다음 로드맵 단계) - 지금은 앱별로 "언제 무엇을 어디에 냈는지"만 수동으로 남긴다.

/// 릴리스가 올라가는 채널 - 닫힌 집합.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseChannel {
    AppStore,
    PlayStore,
    Github,
    Other,
}

/// 릴리스 진행 상태 - 닫힌 집합. released 로의 전이는 releasedAt 자동 스탬프(아래 ReleaseRecord 문서,
/// commands.rs::update_release)의 트리거이기도 하다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseStatus {
    Preparing,
    Submitted,
    Approved,
    Rejected,
    Released,
}

/// 등록된 릴리스 기록 한 건 - app config dir 의 releases.json 에 배열로 저장된다(releases.rs).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseRecord {
    pub id: String,
    pub project_id: String,
    pub version: String,
    pub build_number: Option<String>,
    pub channel: ReleaseChannel,
    pub status: ReleaseStatus,
    /// 자유 텍스트(단순 textarea) - 심사 코멘트, 체크리스트 등. 구조화하지 않는다(1차 범위 밖).
    pub notes: String,
    pub created_at: String,
    pub updated_at: String,
    /// status 가 Released 로 "처음" 바뀐 시점(RFC3339) - commands.rs::update_release 가 자동으로
    /// 채운다(이미 released 인 레코드를 다시 저장해도 갱신되지 않는다, "처음" 전이만). 그 전까지는 None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub released_at: Option<String>,
}

/// get_project_current_version 커맨드 반환 형태 - 새 릴리스 폼의 버전 pre-fill 에 쓴다. 등록 시점
/// 스냅샷(ProjectRecord::version/build_number)이 아니라 pubspec.yaml 을 지금 다시 읽은 값이다.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCurrentVersion {
    pub version: Option<String>,
    pub build_number: Option<String>,
}

// ── CLI 명령 문서(3단계, bildorak-cli) ────────────────────────────────────────
// CLI 서브커맨드 설명의 단일 소스 - build.rs::cli_manifest() 가 이 타입으로 목록을 만든다(model.rs 는
// 이 파일 전체 원칙대로 데이터 모양만 정의). GUI 설정 화면의 "CLI / 자동화" 섹션(SettingsView.tsx)이
// get_cli_manifest 커맨드(commands.rs)로 같은 목록을 그대로 재사용한다 - 소비처는 bin/cli.rs 문서 주석
// (사람이 읽는 --help 안내)과 이 GUI 화면 둘이다.

/// CLI 서브커맨드 하나의 설명 - name/args 는 사람이 터미널에 그대로 타이핑하는 형태를 그대로 담는다
/// (고정 문자열, 실행에는 쓰이지 않는다 - 실제 인자 파싱은 bin/cli.rs 의 clap 구조체가 담당).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliCommandDoc {
    pub name: String,
    pub args: String,
    pub description: String,
    pub example: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FoundKeyKind 의 "type" 태그 값 + 필드명이 실제로 프론트(types.ts)가 기대하는 문자열 그대로
    /// 나오는지 실측 고정한다 - serde 의 자동 케이스 변환을 눈으로만 검증하면 "P8" 같은 숫자 포함
    /// variant 이름에서 틀리기 쉬워서다(위 FoundKeyKind 주석 참고).
    #[test]
    fn found_key_kind_serializes_with_explicit_type_tags() {
        let android = FoundKeyKind::AndroidKeystore {
            alias: Some("release".to_string()),
            key_properties_path: None,
            passwords_available: true,
            app_id: Some("com.example.myapp".to_string()),
        };
        let json = serde_json::to_string(&android).expect("직렬화 실패하면 안 된다");
        assert!(json.contains("\"type\":\"android_keystore\""), "실제 JSON: {json}");
        assert!(json.contains("\"passwordsAvailable\":true"), "실제 JSON: {json}");
        assert!(json.contains("\"keyPropertiesPath\":null"), "실제 JSON: {json}");
        assert!(json.contains("\"appId\":\"com.example.myapp\""), "실제 JSON: {json}");

        let p8 = FoundKeyKind::AppleP8 { key_id: "ABC123DEFG".to_string(), subtype: P8Subtype::AppStoreConnectApi };
        let json = serde_json::to_string(&p8).expect("직렬화 실패하면 안 된다");
        assert!(json.contains("\"type\":\"apple_p8\""), "실제 JSON: {json}");
        assert!(json.contains("\"keyId\":\"ABC123DEFG\""), "실제 JSON: {json}");
        assert!(json.contains("\"subtype\":\"app_store_connect_api\""), "실제 JSON: {json}");
    }

    #[test]
    fn p8_subtype_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&P8Subtype::AppStoreConnectApi).unwrap(), "\"app_store_connect_api\"");
        assert_eq!(serde_json::to_string(&P8Subtype::Subscription).unwrap(), "\"subscription\"");
        assert_eq!(serde_json::to_string(&P8Subtype::Unknown).unwrap(), "\"unknown\"");
    }

    /// ReleaseChannel 의 snake_case 직렬화 문자열을 실측 고정한다 - 프론트(types.ts::ReleaseChannel)가
    /// 이 문자열 그대로를 유니언 타입으로 기대한다.
    #[test]
    fn release_channel_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&ReleaseChannel::AppStore).unwrap(), "\"app_store\"");
        assert_eq!(serde_json::to_string(&ReleaseChannel::PlayStore).unwrap(), "\"play_store\"");
        assert_eq!(serde_json::to_string(&ReleaseChannel::Github).unwrap(), "\"github\"");
        assert_eq!(serde_json::to_string(&ReleaseChannel::Other).unwrap(), "\"other\"");
    }

    /// ReleaseStatus 의 snake_case 직렬화 문자열을 실측 고정한다(프론트 types.ts::ReleaseStatus 와 동일
    /// 목적, 위 release_channel 테스트와 짝).
    #[test]
    fn release_status_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&ReleaseStatus::Preparing).unwrap(), "\"preparing\"");
        assert_eq!(serde_json::to_string(&ReleaseStatus::Submitted).unwrap(), "\"submitted\"");
        assert_eq!(serde_json::to_string(&ReleaseStatus::Approved).unwrap(), "\"approved\"");
        assert_eq!(serde_json::to_string(&ReleaseStatus::Rejected).unwrap(), "\"rejected\"");
        assert_eq!(serde_json::to_string(&ReleaseStatus::Released).unwrap(), "\"released\"");
    }

    /// ReleaseRecord 가 camelCase 필드로 나가고, released_at 이 None 이면 필드 자체가 생략되는지
    /// (skip_serializing_if) 실측 고정한다 - SigningKeyRecord::vault_path 등과 같은 패턴.
    #[test]
    fn release_record_serializes_camel_case_and_omits_unset_released_at() {
        let record = ReleaseRecord {
            id: "r1".to_string(),
            project_id: "p1".to_string(),
            version: "1.0.0".to_string(),
            build_number: Some("7".to_string()),
            channel: ReleaseChannel::AppStore,
            status: ReleaseStatus::Preparing,
            notes: "메모".to_string(),
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            updated_at: "2026-01-01T00:00:00+00:00".to_string(),
            released_at: None,
        };
        let json = serde_json::to_string(&record).expect("직렬화 실패하면 안 된다");
        assert!(json.contains("\"projectId\":\"p1\""), "실제 JSON: {json}");
        assert!(json.contains("\"buildNumber\":\"7\""), "실제 JSON: {json}");
        assert!(json.contains("\"createdAt\":\"2026-01-01T00:00:00+00:00\""), "실제 JSON: {json}");
        assert!(json.contains("\"updatedAt\":\"2026-01-01T00:00:00+00:00\""), "실제 JSON: {json}");
        assert!(!json.contains("releasedAt"), "released_at 이 None 이면 필드 자체가 생략돼야 한다: {json}");

        let released = ReleaseRecord { released_at: Some("2026-02-01T00:00:00+00:00".to_string()), ..record };
        let json = serde_json::to_string(&released).expect("직렬화 실패하면 안 된다");
        assert!(json.contains("\"releasedAt\":\"2026-02-01T00:00:00+00:00\""), "실제 JSON: {json}");
    }
}
