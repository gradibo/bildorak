// key_scan.rs — 서명키/스토어 키 자동 탐색(다음 단계, keychain 이관 옵션 A). signing.rs 가 "이미
// 등록된" 키(경로를 사용자가 직접 고름)를 다루는 반면, 이 파일은 개발 머신 여기저기(홈/다운로드/문서/
// 데스크톱/개발 프로젝트 폴더/CloudStorage)에 흩어진 keystore/.p8 후보를 스스로 찾아낸다(스캔 규칙, 확정된 설계 결정).
//
// ⚠️ 보안 핵심(절대 위반 금지): scan_signing_keys 가 반환하는 FoundKey/FoundKeyKind 어디에도 keystore
// 비밀번호 "값"이 없다 — passwordsAvailable: bool 하나로만 존재 여부를 알린다(model.rs 파일 상단 주석).
// 이 파일 안에서 실제 비밀번호 문자열을 잠깐 들고 있는 KeyPropertiesFound 는 Serialize 를 derive하지
// 않는다 — 구조적으로 어떤 Tauri 커맨드 반환값에도 실릴 수 없다. import_android_signing 만 이 값을
// signing::register_android_signing(keychain 저장)에 바로 넘기고, 그 값 자체를 다시 반환하지 않는다.
//
// 스캔 대상 파일(keystore/key.properties/.p8)은 전부 읽기만 한다 — 어디서도 쓰거나 옮기지 않는다.
//
// 외부 도구를 쓰지 않는다(순수 std::fs 재귀 탐색) — signing.rs/preflight.rs 의 "엔진 원칙"(고정 argv,
// 셸 조립 금지)과 별개로 이 파일은 애초에 자식 프로세스를 띄우지 않는다.

use crate::model::{FoundKey, FoundKeyKind, FoundStoreKeyRecord, ImportAndroidSigningResult, P8Subtype, SigningKeyRecord};
use crate::signing;
use crate::store::write_json_atomic;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const STORE_KEYS_FILE: &str = "store_keys.json";

/// 스캔 시 건너뛸 디렉터리 이름(고정 목록, 스캔 규칙) — 대소문자 정확히 이 스펠링만 본다.
const SKIP_DIR_NAMES: &[&str] =
    &["node_modules", "Caches", "caches", ".git", ".Trash", "DerivedData", ".cache"];

/// 스캔 루트 하나 — path 는 항상 $HOME 기준 절대경로, max_depth 는 "이 디렉터리에서 몇 단계까지 하위
/// 디렉터리로 내려가며 파일을 모을지"(재귀 진입 횟수, walk_dir 문서 참고). $HOME(2)/CloudStorage(6)는
/// 확정된 스펙 값 그대로다. Projects/dev/Developer 등 프로젝트성 루트도 같은 6을 쓴다(모노레포처럼
/// 깊은 위치의 keystore 도 커버). Downloads/Documents/Desktop(4)은 스펙에 명시된 값이 없어 "압축 풀린
/// zip 안에 프로젝트 폴더가 한 겹 더 있는" 흔한 경우까지 넉넉히 잡은 값이다(가정 — 설계 노트로 남김).
struct ScanRoot {
    path: PathBuf,
    max_depth: usize,
}

fn scan_roots(home: &Path) -> Vec<ScanRoot> {
    vec![
        ScanRoot { path: home.to_path_buf(), max_depth: 2 },
        ScanRoot { path: home.join("Downloads"), max_depth: 4 },
        ScanRoot { path: home.join("Documents"), max_depth: 4 },
        ScanRoot { path: home.join("Desktop"), max_depth: 4 },
        ScanRoot { path: home.join("Library").join("CloudStorage"), max_depth: 6 },
        // 흔한 개발 프로젝트 위치 — 없는 사용자는 is_dir() 가드로 그냥 건너뛴다(scan() 참고). 모노레포
        // 깊은 위치의 keystore 도 커버하도록 CloudStorage 와 같은 깊이(6)를 쓴다.
        ScanRoot { path: home.join("Projects"), max_depth: 6 },
        ScanRoot { path: home.join("projects"), max_depth: 6 },
        ScanRoot { path: home.join("dev"), max_depth: 6 },
        ScanRoot { path: home.join("Developer"), max_depth: 6 },
        ScanRoot { path: home.join("code"), max_depth: 6 },
        ScanRoot { path: home.join("StudioProjects"), max_depth: 6 },
        ScanRoot { path: home.join("AndroidStudioProjects"), max_depth: 6 },
        ScanRoot { path: home.join("Documents").join("GitHub"), max_depth: 6 },
    ]
}

/// 서명키/스토어 키 후보 스캔 — 고정 스캔 루트만 본다(scan_roots). 루트가 없거나(예: CloudStorage 를
/// 아직 한 번도 안 켜본 계정) 권한 문제로 못 읽어도 나머지 루트는 계속 진행한다(관대한 처리,
/// signing.rs::read_cert_expiry 와 같은 철학) — 하나가 막혔다고 전체를 실패로 만들지 않는다. 여러 루트가
/// 겹치는 경로를 두 번 찾아도(예: $HOME depth 순회가 $HOME/Projects 를 일부 다시 훑는 경우) 결과는
/// 경로 기준으로 한 번만 담는다(seen).
pub fn scan(home: &Path) -> Vec<FoundKey> {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<FoundKey> = Vec::new();
    for root in scan_roots(home) {
        if !root.path.is_dir() {
            continue;
        }
        walk_dir(&root.path, 0, root.max_depth, &mut seen, &mut out);
    }
    // 최근 것부터 보여준다 — 최근에 만들었거나 손댄 키가 지금 등록하려는 키일 확률이 높다.
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

/// dir 안의 파일을 후보로 모으고, dir 의 하위 디렉터리는 depth < max_depth 일 때만 재귀한다 — 즉
/// max_depth 는 "이 루트에서 몇 번 하위 디렉터리로 내려가는 것까지 허용하는지"다(depth 0 = 루트 바로
/// 안의 파일, depth N = 하위 디렉터리를 N 번 내려간 곳의 파일). 심링크는 파일이든 디렉터리든 절대
/// 따라가지 않는다 — 이것이 순환 방지의 전부다(entry.file_type() 은 심링크 자체를 lstat 급으로 보고하고
/// 따라가지 않으므로, is_symlink() 인 항목은 재귀도 후보 수집도 하지 않으면 순환이 원천적으로 불가능).
fn walk_dir(dir: &Path, depth: usize, max_depth: usize, seen: &mut HashSet<PathBuf>, out: &mut Vec<FoundKey>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if SKIP_DIR_NAMES.contains(&name) {
                continue;
            }
            if depth < max_depth {
                walk_dir(&path, depth + 1, max_depth, seen, out);
            }
            continue;
        }
        if file_type.is_file() {
            collect_if_candidate(&path, seen, out);
        }
    }
}

fn collect_if_candidate(path: &Path, seen: &mut HashSet<PathBuf>, out: &mut Vec<FoundKey>) {
    let Some(ext) = path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()) else {
        return;
    };
    if !matches!(ext.as_str(), "jks" | "keystore" | "p8") {
        return;
    }
    if seen.contains(path) {
        return;
    }
    let Ok(metadata) = fs::metadata(path) else { return };
    let Some(found) = build_found_key(path, &metadata, &ext) else { return };
    if found.is_debug {
        // 기본목록 제외(스캔 규칙) — debug.keystore 는 릴리스 서명 후보가 아니다. is_debug 판정
        // 자체는 build_found_key 가 이미 했으니 여기서는 그 결과로 걸러내기만 한다.
        return;
    }
    seen.insert(path.to_path_buf());
    out.push(found);
}

fn build_found_key(path: &Path, metadata: &fs::Metadata, ext: &str) -> Option<FoundKey> {
    let size = metadata.len();
    let modified = format_modified(metadata);
    match ext {
        "jks" | "keystore" => {
            let file_name = path.file_name()?.to_str()?;
            let is_debug = file_name.eq_ignore_ascii_case("debug.keystore");
            let props = find_key_properties(path);
            let app_id = find_app_id(path, props.as_ref().and_then(|p| Path::new(p.path.as_str()).parent()));
            let kind = FoundKeyKind::AndroidKeystore {
                alias: props.as_ref().and_then(|p| p.key_alias.clone()),
                key_properties_path: props.as_ref().map(|p| p.path.clone()),
                passwords_available: props.as_ref().map(KeyPropertiesFound::has_both_passwords).unwrap_or(false),
                app_id,
            };
            Some(FoundKey { path: path.to_string_lossy().to_string(), kind, size, modified, is_debug })
        }
        "p8" => {
            let stem = path.file_stem()?.to_str()?;
            let (key_id, subtype) = parse_p8_filename(stem);
            let kind = FoundKeyKind::AppleP8 { key_id, subtype };
            Some(FoundKey { path: path.to_string_lossy().to_string(), kind, size, modified, is_debug: false })
        }
        _ => None,
    }
}

fn format_modified(metadata: &fs::Metadata) -> String {
    metadata
        .modified()
        .ok()
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// AuthKey_<KEYID>.p8 → App Store Connect API, SubscriptionKey_<KEYID>.p8 → 구독(인앱결제) API, 그 외는
/// 파일명 전체를 key_id 로 두고 Unknown(Apple 개발자 포털이 실제로 내려주는 두 접두사 규칙).
fn parse_p8_filename(stem: &str) -> (String, P8Subtype) {
    if let Some(rest) = stem.strip_prefix("AuthKey_") {
        return (rest.to_string(), P8Subtype::AppStoreConnectApi);
    }
    if let Some(rest) = stem.strip_prefix("SubscriptionKey_") {
        return (rest.to_string(), P8Subtype::Subscription);
    }
    (stem.to_string(), P8Subtype::Unknown)
}

// ── key.properties 탐색/파싱 ──────────────────────────────────────────────────────────────
// ⚠️ 이 구조체는 Serialize 를 derive 하지 않는다 — 비밀번호 원문을 담고 있어 어떤 Tauri 커맨드 반환값
// (FoundKey 등)에도 구조적으로 실릴 수 없다(파일 상단 보안 주석). has_both_passwords() 로 존재 여부
// bool 만 뽑아 FoundKeyKind::AndroidKeystore::passwords_available 에 투영한다.

struct KeyPropertiesFound {
    /// 이 key.properties 파일 자체의 경로 — 비밀 아님, FoundKeyKind::AndroidKeystore::
    /// key_properties_path 로 그대로 나간다.
    path: String,
    /// store_file_matches 검증에만 쓰고 그 뒤로는 쓰지 않는다(가벼운 문자열이라 계속 들고 있어도 비용
    /// 없음 — 구조체를 파싱용/검증후용 두 개로 나누지 않으려고 그대로 둔다).
    store_file: Option<String>,
    key_alias: Option<String>,
    store_password: Option<String>,
    key_password: Option<String>,
}

impl KeyPropertiesFound {
    fn has_both_passwords(&self) -> bool {
        Self::non_empty(&self.store_password) && Self::non_empty(&self.key_password)
    }

    fn non_empty(value: &Option<String>) -> bool {
        value.as_deref().is_some_and(|s| !s.trim().is_empty())
    }
}

/// key.properties 한 줄 형식(key=value)만 이해한다 — Java Properties 의 콜론 구분자·줄 이어붙이기·
/// 유니코드 이스케이프는 다루지 않는다(Flutter 공식 예제가 항상 이 단순 형태라 실사용에 충분하다,
/// 관대한 처리 철학 — 못 읽은 필드는 그냥 None 이 되고 하드 에러로 이어지지 않는다).
fn parse_key_properties(path: &Path) -> Option<KeyPropertiesFound> {
    let raw = fs::read_to_string(path).ok()?;
    let mut store_file = None;
    let mut key_alias = None;
    let mut store_password = None;
    let mut key_password = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else { continue };
        let value = value.trim().to_string();
        match key.trim() {
            "storeFile" => store_file = Some(value),
            "keyAlias" => key_alias = Some(value),
            "storePassword" => store_password = Some(value),
            "keyPassword" => key_password = Some(value),
            _ => {}
        }
    }
    Some(KeyPropertiesFound {
        path: path.to_string_lossy().to_string(),
        store_file,
        key_alias,
        store_password,
        key_password,
    })
}

/// keystore 와 같은 디렉터리, 또는 그 부모 디렉터리에서 key.properties 를 찾는다 — 이 머신의 실제
/// 배치는 "각 keystore 옆 key.properties"(같은 디렉터리)이고, 부모 디렉터리 후보는 전통적 Flutter
/// 레이아웃(android/key.properties + android/app/release.jks)까지 넓게 보기 위한 보조 후보다. 후보를
/// 찾아도 store_file_matches 로 다시 검증한다 — 같은 폴더에 keystore 가 여러 개면 엉뚱한
/// key.properties 를 잘못 연결할 수 있어서다.
fn find_key_properties(keystore_path: &Path) -> Option<KeyPropertiesFound> {
    let dir = keystore_path.parent()?;
    let mut candidates = vec![dir.join("key.properties")];
    if let Some(parent) = dir.parent() {
        candidates.push(parent.join("key.properties"));
    }
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        let Some(parsed) = parse_key_properties(&candidate) else { continue };
        if !store_file_matches(parsed.store_file.as_deref(), &candidate, keystore_path) {
            continue;
        }
        return Some(parsed);
    }
    None
}

/// key.properties 를 keystore 옆/부모 디렉터리가 아니라 "이 keystore 가 속한 프로젝트 자체"에서 찾는다 —
/// repo_path/android/key.properties(find_app_id_in_project_dir 와 같은 경로 규칙, commands.rs::
/// get_project_app_id 가 이미 "<repo_path>/android" 를 android 프로젝트 루트로 쓴다). 홑파일 keystore
/// (원본 옆에 key.properties 가 없어 signing.rs::build_record 만으로 등록된 경우)를 이미 등록된 앱에
/// 연결한 "다음" 이 경로에서 비밀번호를 찾는 용도다(autofill_android_signing_from_project 가 호출).
/// find_key_properties 와 달리 store_file_matches_strict 로 검증한다(관대한 전역 store_file_matches 가
/// 아니다) — 그 프로젝트의 key.properties 가 가리키는 keystore 가 실제로 이 keystore_path 와 다르거나
/// (예: 그 앱에 이미 다른 release.jks 가 연결돼 있는 상태에서 별개의 keystore 를 새로 등록하는 경우)
/// keystore_path 자체가 이동/삭제돼 canonicalize 가 실패하면(리뷰 지적 — stale 원본, 리뷰에서 강조된
/// "엉뚱한 비번 방지") 엉뚱한 비밀번호를 잘못 연결하지 않는다 — 그 경우 None 을 돌려주고 호출부가 수동
/// 입력으로 폴백한다.
fn find_project_key_properties(repo_path: &Path, keystore_path: &Path) -> Option<KeyPropertiesFound> {
    let candidate = repo_path.join("android").join("key.properties");
    if !candidate.is_file() {
        return None;
    }
    let parsed = parse_key_properties(&candidate)?;
    if !store_file_matches_strict(parsed.store_file.as_deref(), &candidate, keystore_path) {
        return None;
    }
    Some(parsed)
}

/// key.properties 의 storeFile 값이 실제로 이 keystore_path 를 가리키는지 확인한다 — 같은 폴더에 여러
/// keystore 가 있을 때 엉뚱한 key.properties 를 잘못 연결해 버리는 사고를 막는 안전장치다. storeFile 이
/// 절대경로면(Flutter 공식 예제가 권장하는 방식) 그대로 정확히 비교되고, key.properties 와 같은
/// 디렉터리 기준 상대경로면(이 머신의 실제 배치 — "각 keystore 옆 key.properties") 그 디렉터리 기준으로
/// 맞춰 비교한다.
/// ⚠️ 알려진 한계: key.properties 가 keystore 의 "부모" 디렉터리에 있고 storeFile 이 그 부모가 아니라
/// 별도 app 모듈 폴더 기준 상대경로인 전통적 Gradle 배치(예: android/key.properties +
/// storeFile=upload-keystore.jks 가 android/app/ 기준)는 여기서 불일치로 판정될 수 있다 — 그 경우
/// 안전하게 "확인 불가"로 물러나 수동 입력 폴백으로 이어질 뿐, 잘못된 keystore 에 비밀번호를 잘못
/// 연결하는 사고는 나지 않는다(알려진 비차단 한계로 문서화).
/// storeFile 자체가 없으면(구버전/수동 설정 등) 비교할 근거가 없으므로 보수적으로 "일치"로 본다 —
/// 그렇지 않으면 정상 케이스(단일 keystore + key.properties, storeFile 생략)까지 막아버린다.
fn store_file_matches(store_file: Option<&str>, key_properties_path: &Path, keystore_path: &Path) -> bool {
    let Some(store_file) = store_file else { return true };
    let Some(base_dir) = key_properties_path.parent() else { return true };
    let resolved = base_dir.join(store_file);
    match (resolved.canonicalize(), keystore_path.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => true,
    }
}

/// store_file_matches 의 autofill 전용 엄격(fail-closed) 버전 — find_project_key_properties(autofill_
/// android_signing_from_project 가 호출하는 유일한 경로) 안에서만 쓴다. store_file_matches 는 "storeFile
/// 이 없거나 canonicalize 가 실패하면 비교 근거가 없으니 관대하게 일치로 본다"는 전제인데, 그 전제는
/// find_key_properties 문맥(key.properties 가 keystore 와 같은/부모 디렉터리에 있어 위치 자체가 이미
/// 강한 신호)에서만 성립한다. autofill 경로는 그 전제가 성립하지 않는다 — repo_path/android/
/// key.properties 는 keystore 원본과 디렉터리 관계가 전혀 없을 수 있다(원본이 프로젝트 밖 홑파일로
/// 등록된 경우가 기본 시나리오). 그 상태에서 원본을 이동/삭제하면 keystore_path.canonicalize() 가
/// 실패해 관대한 `_ => true` 로 빠지고, 프로젝트 key.properties 가 가리키는 "다른" keystore 의
/// 비밀번호를 이 레코드에 잘못 이관하는 사고로 이어진다(리뷰 지적, 리뷰에서 강조된 "엉뚱한 비번 방지").
/// 그래서 이 버전은 storeFile 자체가 없거나 canonicalize 가 어느 한쪽이라도 실패하면 무조건 false —
/// 원본·storeFile 둘 다 실재해서 canonicalize 로 정확히 같은 경로로 확인될 때만 true(관대한 기본값
/// 없음, "애매하면 폴백"). ⚠️ store_file_matches(전역)는 이 변경과 무관하게 그대로 유지한다 —
/// find_key_properties 를 통한 기존 인접 key.properties import 동작을 바꾸지 않는다(이번 변경 범위 밖).
fn store_file_matches_strict(store_file: Option<&str>, key_properties_path: &Path, keystore_path: &Path) -> bool {
    let Some(store_file) = store_file else { return false };
    let Some(base_dir) = key_properties_path.parent() else { return false };
    let resolved = base_dir.join(store_file);
    match (resolved.canonicalize(), keystore_path.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

// ── 앱 라벨(applicationId) 추정 ────────────────────────────────────────────────────────────
// key.properties/keystore 위치 근처의 안드로이드 프로젝트 build.gradle(.kts)에서 applicationId(우선)
// 또는 namespace(폴백)를 읽어 "이 키가 어느 앱 것인지" 라벨을 붙인다. build.gradle 은 key.properties 와
// 달리 비밀번호를 담지 않는다 — 이 값은 FoundKeyKind::AndroidKeystore::app_id 로 스캔 결과에 그대로
// 실어도 안전하다(비번 필요 없이 항상 시도).
//
// 실측된 배치 3가지(확정된 설계, 전부 "key.properties 가 있는 디렉터리 기준 app/ 서브폴더에 build.gradle"
// 로 통일된다):
//   - Flutter(이 머신 실측): <proj>/app/android/key.properties →
//     <proj>/app/android/app/build.gradle.kts, `applicationId = "com.example.myapp"`(Kotlin DSL, 등호 +
//     큰따옴표).
//   - 모노레포(이 머신 실측): <proj>/apps/mobile/android/app/build.gradle,
//     `applicationId 'com.example.otherapp'`(Groovy, 공백 구분 + 홑따옴표).
//   - 일반/RN: <proj>/android/key.properties → <proj>/android/app/build.gradle.

/// key.properties 위치 기준으로 우선 내려가 볼 하위 폴더 이름 — 위 3가지 실측 배치 전부 이 이름
/// 그대로다(Android Gradle 프로젝트의 관례적 app 모듈 이름).
const GRADLE_APP_MODULE_DIR: &str = "app";

/// key.properties 를 못 찾아 keystore 경로만으로 프로젝트 루트를 추정할 때(find_app_id_by_climbing) 몇
/// 단계까지 상위로 올라가며 찾을지 — scan_roots 의 프로젝트성 루트 깊이(6)만큼 넉넉히 잡아 모노레포
/// 깊은 위치의 keystore 도 커버한다(정확한 상한을 정한 스펙은 없다 — 가정, 설계 노트로 남김).
const APP_ID_CLIMB_MAX_LEVELS: usize = 6;

/// keystore/key.properties 위치에서 가까운 안드로이드 프로젝트의 build.gradle(.kts)을 찾아 applicationId
/// (우선) 또는 namespace(폴백)를 파싱한다. key_properties_dir 이 있으면(find_key_properties 가 이미 이
/// keystore 와 storeFile 일치를 확인해 둔 디렉터리) 그 디렉터리의 app/build.gradle{.kts,}만 먼저 본다 —
/// 이미 신뢰할 수 있는 위치라 다른 곳을 더 뒤질 필요가 없다. 없거나 거기서 못 찾았으면 keystore 경로
/// 자체에서 상위로 올라가며 "<조상>/android/app/build.gradle{.kts,}" 를 찾는다(스캔으로 keystore 만
/// 발견되고 key.properties 는 못 찾거나 storeFile 이 불일치했던 경우의 대비). 끝까지 못 찾으면 조용히
/// None — 앱 라벨만 안 붙을 뿐 스캔 자체는 계속된다(관대한 처리, 파일 전체 철학과 동일).
fn find_app_id(keystore_path: &Path, key_properties_dir: Option<&Path>) -> Option<String> {
    if let Some(dir) = key_properties_dir {
        if let Some(app_id) = find_app_id_in_project_dir(dir) {
            return Some(app_id);
        }
    }
    find_app_id_by_climbing(keystore_path)
}

/// dir(예: <proj>/app/android/ 또는 <proj>/android/)의 app/build.gradle.kts 또는 app/build.gradle 을
/// 읽어 applicationId/namespace 를 파싱한다. 둘 다 있으면 .kts 를 먼저 본다(순서에 의미는 없다 — 한
/// 프로젝트에 보통 둘 중 하나만 있다).
///
/// pub — 스캔 경로(find_app_id) 뿐 아니라 commands.rs::get_project_app_id 도 그대로 재사용한다(서명키
/// 체크리스트 화면의 앱 라벨). 이미 등록된 프로젝트는 repo_path(pubspec.yaml 루트)를 알고 있어 climbing
/// 없이 곧장 "<repo_path>/android" 를 dir 로 넘기면 된다 — pubspec.rs::detect_platforms 가 Android
/// 플랫폼을 "<repo_path>/android" 존재로 감지하는 것과 동일한 경로라 실측상 안전하다.
pub fn find_app_id_in_project_dir(dir: &Path) -> Option<String> {
    let app_dir = dir.join(GRADLE_APP_MODULE_DIR);
    for file_name in ["build.gradle.kts", "build.gradle"] {
        let candidate = app_dir.join(file_name);
        if candidate.is_file() {
            if let Some(app_id) = parse_application_id(&candidate) {
                return Some(app_id);
            }
        }
    }
    None
}

/// keystore_path 의 조상 디렉터리를 최대 APP_ID_CLIMB_MAX_LEVELS 단계까지 올라가며 "<조상>/android/app/
/// build.gradle{.kts,}" 를 찾는다. 예: keystore 가 <proj>/apps/mobile/android/release.jks 에 있으면
/// <proj>/apps/mobile 까지 1단계만 올라가도 <proj>/apps/mobile/android/app/build.gradle 을 찾는다.
fn find_app_id_by_climbing(keystore_path: &Path) -> Option<String> {
    let mut dir = keystore_path.parent();
    for _ in 0..=APP_ID_CLIMB_MAX_LEVELS {
        let current = dir?;
        if let Some(app_id) = find_app_id_in_project_dir(&current.join("android")) {
            return Some(app_id);
        }
        dir = current.parent();
    }
    None
}

/// build.gradle/build.gradle.kts 한 줄에서 applicationId(우선) 또는 namespace(폴백) 값을 읽는다. Groovy
/// (`applicationId 'x'` 공백 구분, 또는 `applicationId = 'x'`)와 Kotlin DSL(`applicationId = "x"`) 둘 다
/// 이해한다 — 등호는 있어도 없어도 되고, 따옴표는 홑/겹 둘 다 허용한다. 줄 전체가 주석(`//`)이면
/// 건너뛴다. applicationId 가 없으면 namespace 로 폴백(파일 안에서 어느 줄이 먼저 나오든 applicationId
/// 를 항상 우선한다 — 두 값이 다르면 실제 배포 식별자는 applicationId 다).
fn parse_application_id(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    find_gradle_string_field(&raw, "applicationId").or_else(|| find_gradle_string_field(&raw, "namespace"))
}

/// raw 안에서 `<field> [=] '값'` 또는 `<field> [=] "값"` 형태의 첫 줄을 찾아 값만 뽑는다. field 바로
/// 뒤가 공백/`=`/따옴표가 아니면(예: applicationIdSuffix 처럼 식별자가 그대로 이어짐) extract_quoted 가
/// 첫 글자에서 바로 실패해 오탐 없이 다음 줄로 넘어간다.
fn find_gradle_string_field(raw: &str, field: &str) -> Option<String> {
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix(field) else { continue };
        let rest = rest.trim_start();
        let rest = rest.strip_prefix('=').map(str::trim_start).unwrap_or(rest);
        if let Some(value) = extract_quoted(rest) {
            return Some(value);
        }
    }
    None
}

/// 문자열 s 가 따옴표(홑/겹)로 시작하면 그 안쪽 값을 뽑는다. 시작이 따옴표가 아니거나 닫는 따옴표가
/// 없으면 None(하드 에러 아님 — find_gradle_string_field 가 다음 줄로 넘어간다).
fn extract_quoted(s: &str) -> Option<String> {
    let mut chars = s.chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = chars.as_str();
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

// ── Android keystore 가져오기(옵션 A: 비밀번호 자동 keychain 이관) ────────────────────────────

/// scan() 으로 찾은 Android keystore 를 등록 + 프로젝트에 연결하고, key.properties 에서 비밀번호까지
/// 찾으면 signing::register_android_signing 으로 keychain 에 자동 이관한다(옵션 A). 이미 등록된
/// 경로(signing_keys.json 에 file_path 가 같은 레코드가 있음)면 새로 만들지 않고 그 레코드를 갱신한다
/// (스캔 결과를 여러 번 눌러도 중복 레코드가 쌓이지 않게). key.properties 가 없거나 비밀번호 필드가
/// 비어 있으면 keychain 은 건드리지 않고 등록·연결까지만 하고 imported:false 로 그 사실만 알린다
/// (commands.rs::import_found_android_signing 이 프론트에 그대로 전달 — 프론트가 수동 입력 폼으로
/// 폴백). 프론트가 스캔 시점에 봤던 passwordsAvailable 값을 그대로 믿지 않고 여기서 key.properties 를
/// 다시 읽는다(TOCTOU 방지 — 스캔과 클릭 사이에 파일이 바뀔 수 있다). 원본 keystore/key.properties
/// 파일은 어디서도 쓰지 않는다 — signing::build_record 와 find_key_properties 둘 다 읽기 전용.
pub fn import_android_signing(
    base_dir: &Path,
    vault_dir: &Path,
    keystore_path: &Path,
    project_id: &str,
) -> Result<ImportAndroidSigningResult, String> {
    let mut keys = signing::load_signing_keys(base_dir)?;
    let path_str = keystore_path.to_string_lossy().to_string();
    let existing_index = keys.iter().position(|k| k.file_path == path_str);

    let mut record = match existing_index {
        Some(idx) => keys[idx].clone(),
        None => signing::build_record(keystore_path)?,
    };
    if record.kind != crate::model::SigningKeyKind::AndroidKeystore {
        return Err("Android keystore 파일만 가져올 수 있어요.".to_string());
    }
    // 안전 보관 볼트로 복사(확정된 설계 결정, keystore 분실 대비 백업) — 이미 볼트 사본이 있으면(다른
    // 프로젝트에 재사용하려고 다시 가져오는 경우) 새로 복사하지 않는다(register_signing_key 와 동일한
    // "새로 만들 때만" 원칙, signing.rs::copy_keystore_into_vault 문서 참고).
    if record.vault_path.is_none() {
        let vault_path = signing::copy_keystore_into_vault(vault_dir, keystore_path, &record.id)?;
        record.vault_path = Some(vault_path.to_string_lossy().to_string());
    }
    if !record.linked_project_ids.iter().any(|id| id == project_id) {
        record.linked_project_ids.push(project_id.to_string());
    }

    // 인증서 겉정보(keytool) 재조회는 볼트 사본을 우선 쓴다(자체 완결 원칙, model.rs::SigningKeyRecord::
    // vault_path 문서 참고) — key.properties 탐색 자체는 항상 원본 keystore_path 기준이다(storeFile 이
    // 볼트 경로를 가리킬 리 없다, 아래 apply_found_key_properties 와 역할이 다르다).
    let registration_path =
        record.vault_path.as_deref().map(PathBuf::from).unwrap_or_else(|| keystore_path.to_path_buf());
    let props = find_key_properties(keystore_path);
    let (imported, key_alias) = apply_found_key_properties(&registration_path, &mut record, props)?;

    match existing_index {
        Some(idx) => keys[idx] = record.clone(),
        None => keys.push(record.clone()),
    }
    signing::save_signing_keys(base_dir, &keys)?;

    Ok(ImportAndroidSigningResult { key: record, imported, key_alias })
}

/// key.properties(어디서 찾았든)에서 비밀번호를 찾았으면 keychain 에 등록하고 record.android_signing 을
/// 채운다 — import_android_signing(keystore 옆/부모 디렉터리 탐색)과
/// autofill_android_signing_from_project(프로젝트 자체 key.properties 탐색)가 "어디서 찾는지"만 다르고
/// "찾은 다음 무엇을 하는지"는 완전히 같아서 이 부분만 공유한다. registration_keystore_path 는 인증서
/// 겉정보 추출(keytool)에 실제로 열어 볼 파일 — 볼트 복사가 있으면 볼트 사본(자체 완결 원칙), 없으면
/// 원본이다(호출부가 결정해 넘긴다, 이 함수는 "어느 경로인지" 모른다). 반환하는 key_alias 는 imported
/// 여부와 무관하게 항상 있으면 채워 돌려준다 — 실패해도 프론트 수동 폼 pre-fill 용으로 쓰인다.
fn apply_found_key_properties(
    registration_keystore_path: &Path,
    record: &mut SigningKeyRecord,
    props: Option<KeyPropertiesFound>,
) -> Result<(bool, Option<String>), String> {
    let props = props.filter(KeyPropertiesFound::has_both_passwords);
    let key_alias =
        props.as_ref().and_then(|p| p.key_alias.clone()).filter(|alias| !alias.trim().is_empty());
    let imported = if let (Some(props), Some(alias)) = (&props, &key_alias) {
        let config = signing::register_android_signing(
            registration_keystore_path,
            &record.id,
            alias,
            props.store_password.as_deref().unwrap_or_default(),
            props.key_password.as_deref().unwrap_or_default(),
        )?;
        record.android_signing = Some(config);
        true
    } else {
        false
    };
    Ok((imported, key_alias))
}

// ── 프로젝트 자체 key.properties 자동 채움(홑파일 keystore, 등록 당시 옆에 key.properties 없음) ────

/// 홑파일 keystore(등록 당시 옆에 key.properties 가 없어 signing.rs::build_record 만으로 등록된 경우)를
/// 프로젝트에 연결한 "다음" 시도하는 비밀번호 자동 채움(확정된 설계 결정) — 그 프로젝트 자체의
/// key.properties(find_project_key_properties, "<repo_path>/android/key.properties")에서
/// storePassword/keyPassword/keyAlias 를 찾아 storeFile 이 이 keystore 를 정확히 가리킬 때만(안전 매칭,
/// store_file_matches) keychain 에 자동 이관한다. import_android_signing(스캔 결과 가져오기, key.properties
/// 를 keystore 옆/부모에서 찾음)과는 "어디서 찾는지"만 다르고 나머지는 apply_found_key_properties 를
/// 그대로 공유한다. commands.rs::register_signing_key + link_signing_key 로 이미 등록·연결까지 끝난
/// 키에만 쓴다(key_id 로 기존 레코드를 찾는다 — 새로 만들지 않는다, import_android_signing 과 달리 이
/// 함수는 build_record 를 호출하지 않는다). 불일치/파일없음이면 imported:false 를 돌려주고 호출부
/// (프론트)가 기존처럼 수동 입력 폼으로 폴백한다 — 추측 금지.
pub fn autofill_android_signing_from_project(
    base_dir: &Path,
    repo_path: &Path,
    key_id: &str,
) -> Result<ImportAndroidSigningResult, String> {
    let mut keys = signing::load_signing_keys(base_dir)?;
    let index = keys
        .iter()
        .position(|k| k.id == key_id)
        .ok_or_else(|| "등록된 서명키를 찾지 못했어요.".to_string())?;
    let mut record = keys[index].clone();
    if record.kind != crate::model::SigningKeyKind::AndroidKeystore {
        return Err("Android keystore 파일에만 자동 채움을 시도할 수 있어요.".to_string());
    }
    let original_path = PathBuf::from(&record.file_path);
    // register_signing_key 가 이미 이 레코드를 볼트에 복사해 뒀어야 정상이다(kind == AndroidKeystore 는
    // 등록 시점에 항상 채운다, model.rs 문서 참고) — 혹시 없으면(이 기능 이전 레코드 등) 원본으로
    // 안전하게 물러난다.
    let registration_path =
        record.vault_path.as_deref().map(PathBuf::from).unwrap_or_else(|| original_path.clone());

    // fail-closed 가드(리뷰 지적, 리뷰에서 강조된 "엉뚱한 비번 방지") — record.file_path 는 등록 시점
    // 경로 그대로라 stale 할 수 있다(원본을 이동/삭제해도 레코드는 그대로 남는다). 원본이 실재하지
    // 않으면 storeFile 매칭 자체를 시도하지 않는다 — find_project_key_properties 안의
    // store_file_matches_strict 도 canonicalize 실패를 불일치로 처리해 결과는 같지만, 여기서 먼저 끊어
    // "원본이 없으면 애초에 시도하지 않는다"는 의도를 호출부에서 명시적으로 드러낸다.
    let props =
        if original_path.is_file() { find_project_key_properties(repo_path, &original_path) } else { None };
    let (imported, key_alias) = apply_found_key_properties(&registration_path, &mut record, props)?;

    keys[index] = record.clone();
    signing::save_signing_keys(base_dir, &keys)?;
    Ok(ImportAndroidSigningResult { key: record, imported, key_alias })
}

// ── .p8 발견 기록(가벼움 — keychain 이관 없음) ─────────────────────────────────────────────

fn store_keys_file_path(base_dir: &Path) -> PathBuf {
    base_dir.join(STORE_KEYS_FILE)
}

/// signing.rs::load_signing_keys 와 완전히 동일한 규칙(파일 없으면 빈 목록) — 별도 파일
/// (store_keys.json)에 .p8 "발견 기록"만 담는다. 아직 소비처가 없는 가벼운 기록이라
/// signing_keys.json 과 섞지 않고 관심사를 분리했다(다음 로드맵 단계가 나중에 읽는다).
pub fn load_found_store_keys(base_dir: &Path) -> Result<Vec<FoundStoreKeyRecord>, String> {
    let path = store_keys_file_path(base_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("기록된 스토어 키 목록을 읽지 못했어요: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&raw).map_err(|e| format!("기록된 스토어 키 목록이 손상됐어요: {e}"))
}

/// signing.rs::save_signing_keys 와 동일 규칙(pretty JSON + write_json_atomic).
pub fn save_found_store_keys(base_dir: &Path, keys: &[FoundStoreKeyRecord]) -> Result<(), String> {
    let path = store_keys_file_path(base_dir);
    let raw = serde_json::to_string_pretty(keys).map_err(|e| format!("저장할 데이터를 만들지 못했어요: {e}"))?;
    write_json_atomic(&path, &raw).map_err(|e| format!("스토어 키 기록을 저장하지 못했어요: {e}"))
}

/// .p8 "발견 기록"을 저장한다 — keychain 을 건드리지 않는다(파일 상단 주석). 같은 path 가 이미
/// 기록돼 있으면 새로 만들지 않고 기존 레코드를 그대로 돌려준다(멱등 — 스캔 결과에서 여러 번 눌러도
/// 중복이 쌓이지 않는다).
pub fn register_found_store_key(
    base_dir: &Path,
    path: &str,
    key_id: &str,
    subtype: P8Subtype,
) -> Result<FoundStoreKeyRecord, String> {
    let mut keys = load_found_store_keys(base_dir)?;
    if let Some(existing) = keys.iter().find(|k| k.path == path) {
        return Ok(existing.clone());
    }
    let record = FoundStoreKeyRecord {
        id: uuid::Uuid::new_v4().to_string(),
        path: path.to_string(),
        key_id: key_id.to_string(),
        subtype,
        registered_at: chrono::Utc::now().to_rfc3339(),
    };
    keys.push(record.clone());
    save_found_store_keys(base_dir, &keys)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use uuid::Uuid;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bildorak-keyscan-test-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("failed to create temp dir");
        dir
    }

    fn write_file(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn parse_p8_filename_detects_known_prefixes() {
        assert_eq!(
            parse_p8_filename("AuthKey_ABC123DEFG"),
            ("ABC123DEFG".to_string(), P8Subtype::AppStoreConnectApi)
        );
        assert_eq!(
            parse_p8_filename("SubscriptionKey_XYZ987"),
            ("XYZ987".to_string(), P8Subtype::Subscription)
        );
        assert_eq!(parse_p8_filename("SomeOtherName"), ("SomeOtherName".to_string(), P8Subtype::Unknown));
    }

    #[test]
    fn build_found_key_flags_debug_keystore_by_name() {
        let dir = temp_dir("debug-flag");
        let path = dir.join("debug.keystore");
        write_file(&path, b"dummy keystore bytes for test");
        let metadata = fs::metadata(&path).unwrap();
        let found = build_found_key(&path, &metadata, "keystore").expect("빌드 실패하면 안 된다");
        assert!(found.is_debug);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_found_key_normal_jks_is_not_debug() {
        let dir = temp_dir("not-debug");
        let path = dir.join("release.jks");
        write_file(&path, b"dummy keystore bytes for test");
        let metadata = fs::metadata(&path).unwrap();
        let found = build_found_key(&path, &metadata, "jks").expect("빌드 실패하면 안 된다");
        assert!(!found.is_debug);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_key_properties_same_directory() {
        let dir = temp_dir("same-dir");
        let keystore = dir.join("release.jks");
        write_file(&keystore, b"dummy");
        write_file(
            &dir.join("key.properties"),
            b"storeFile=release.jks\nkeyAlias=upload\nstorePassword=storepw\nkeyPassword=keypw\n",
        );
        let found = find_key_properties(&keystore).expect("같은 디렉터리 key.properties 를 찾아야 한다");
        assert_eq!(found.key_alias.as_deref(), Some("upload"));
        assert!(found.has_both_passwords());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_key_properties_parent_directory() {
        let dir = temp_dir("parent-dir");
        let keystore = dir.join("android/app/release.jks");
        write_file(&keystore, b"dummy");
        // storeFile 을 절대경로로 써서(Flutter 공식 예제 권장 방식) 부모 디렉터리 배치에서도
        // store_file_matches 가 확실히 일치를 확인할 수 있게 한다.
        let properties_contents = format!(
            "storeFile={}\nkeyAlias=upload\nstorePassword=storepw\nkeyPassword=keypw\n",
            keystore.to_string_lossy()
        );
        write_file(&dir.join("android/key.properties"), properties_contents.as_bytes());
        let found = find_key_properties(&keystore).expect("부모 디렉터리 key.properties 를 찾아야 한다");
        assert_eq!(found.key_alias.as_deref(), Some("upload"));
        assert!(found.has_both_passwords());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_key_properties_rejects_mismatched_store_file() {
        let dir = temp_dir("mismatch");
        let keystore_a = dir.join("a.jks");
        let keystore_b = dir.join("b.jks");
        write_file(&keystore_a, b"dummy-a");
        write_file(&keystore_b, b"dummy-b");
        // key.properties 는 a.jks 만 가리킨다 — b.jks 로 조회하면 엉뚱하게 연결되면 안 된다.
        write_file(
            &dir.join("key.properties"),
            b"storeFile=a.jks\nkeyAlias=upload\nstorePassword=storepw\nkeyPassword=keypw\n",
        );
        assert!(find_key_properties(&keystore_a).is_some(), "a.jks 는 storeFile 과 일치해야 한다");
        assert!(find_key_properties(&keystore_b).is_none(), "b.jks 는 storeFile 과 불일치해 None 이어야 한다");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_key_properties_missing_passwords_returns_incomplete() {
        let dir = temp_dir("no-pw");
        let keystore = dir.join("release.jks");
        write_file(&keystore, b"dummy");
        write_file(&dir.join("key.properties"), b"storeFile=release.jks\nkeyAlias=upload\n");
        let found = find_key_properties(&keystore).expect("파일은 찾아야 한다");
        assert_eq!(found.key_alias.as_deref(), Some("upload"));
        assert!(!found.has_both_passwords(), "비밀번호 필드가 없으면 false 여야 한다");
        let _ = fs::remove_dir_all(&dir);
    }

    // ── 프로젝트 자체 key.properties(홑파일 keystore 자동 채움) ──────────────────────────────

    #[test]
    fn find_project_key_properties_matches_when_store_file_resolves() {
        // repo_path/android/key.properties — get_project_app_id 와 같은 경로 규칙(파일 상단 문서 참고).
        let dir = temp_dir("project-key-properties-match");
        let repo_path = dir.join("proj");
        let keystore = dir.join("elsewhere/release.jks"); // 홑파일 keystore — 프로젝트 폴더 밖에 있다.
        write_file(&keystore, b"dummy");
        let properties_contents = format!(
            "storeFile={}\nkeyAlias=upload\nstorePassword=storepw\nkeyPassword=keypw\n",
            keystore.to_string_lossy()
        );
        write_file(&repo_path.join("android/key.properties"), properties_contents.as_bytes());

        let found =
            find_project_key_properties(&repo_path, &keystore).expect("프로젝트 key.properties 를 찾아야 한다");
        assert_eq!(found.key_alias.as_deref(), Some("upload"));
        assert!(found.has_both_passwords());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_project_key_properties_rejects_mismatched_store_file() {
        let dir = temp_dir("project-key-properties-mismatch");
        let repo_path = dir.join("proj");
        let registered_keystore = dir.join("registered.jks"); // 지금 등록하려는 keystore.
        let other_keystore = dir.join("other.jks"); // 그 프로젝트에 이미 연결된 다른 keystore.
        write_file(&registered_keystore, b"dummy-registered");
        write_file(&other_keystore, b"dummy-other");
        // key.properties 는 other_keystore 만 가리킨다 — registered_keystore 로 조회하면 엉뚱하게
        // 연결되면 안 된다(안전 매칭, 추측 금지).
        let properties_contents = format!(
            "storeFile={}\nkeyAlias=upload\nstorePassword=storepw\nkeyPassword=keypw\n",
            other_keystore.to_string_lossy()
        );
        write_file(&repo_path.join("android/key.properties"), properties_contents.as_bytes());

        assert!(
            find_project_key_properties(&repo_path, &registered_keystore).is_none(),
            "storeFile 불일치는 None 이어야 한다"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_project_key_properties_none_when_project_has_no_key_properties() {
        let dir = temp_dir("project-key-properties-missing");
        let repo_path = dir.join("proj");
        let keystore = dir.join("release.jks");
        write_file(&keystore, b"dummy");
        fs::create_dir_all(repo_path.join("android")).unwrap(); // android 폴더는 있어도 key.properties 는 없다.
        assert!(find_project_key_properties(&repo_path, &keystore).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    // ── 앱 라벨(applicationId) 추정 ────────────────────────────────────────────────────────

    #[test]
    fn parse_application_id_kotlin_dsl_with_equals_and_double_quotes() {
        // Flutter 실측(app/android/app/build.gradle.kts): 등호 + 큰따옴표.
        let dir = temp_dir("gradle-kts");
        let path = dir.join("build.gradle.kts");
        write_file(
            &path,
            b"namespace = \"com.example.myapp\"\n\ndefaultConfig {\n    applicationId = \"com.example.myapp\"\n}\n",
        );
        assert_eq!(parse_application_id(&path), Some("com.example.myapp".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_application_id_groovy_space_separated_single_quotes() {
        // 모노레포 실측(apps/mobile/android/app/build.gradle): 공백 구분 + 홑따옴표.
        let dir = temp_dir("gradle-groovy");
        let path = dir.join("build.gradle");
        write_file(
            &path,
            b"namespace 'com.example.otherapp'\n\ndefaultConfig {\n    applicationId 'com.example.otherapp'\n}\n",
        );
        assert_eq!(parse_application_id(&path), Some("com.example.otherapp".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_application_id_falls_back_to_namespace_when_applicationid_missing() {
        let dir = temp_dir("gradle-namespace-fallback");
        let path = dir.join("build.gradle.kts");
        write_file(&path, b"namespace = \"com.example.onlynamespace\"\n");
        assert_eq!(parse_application_id(&path), Some("com.example.onlynamespace".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_application_id_ignores_full_line_comments_and_suffix_false_positive() {
        let dir = temp_dir("gradle-comments");
        let path = dir.join("build.gradle.kts");
        write_file(
            &path,
            b"// applicationId = \"com.example.old\"\napplicationIdSuffix = \".debug\"\napplicationId = \"com.example.real\" // release\n",
        );
        // 주석 줄과 applicationIdSuffix(접두 일치 오탐 후보) 는 건너뛰고 실제 값만 뽑아야 한다. 값 뒤
        // 트레일링 주석("// release")은 닫는 따옴표 이후라 애초에 안 읽힌다.
        assert_eq!(parse_application_id(&path), Some("com.example.real".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_application_id_none_when_no_gradle_fields_present() {
        let dir = temp_dir("gradle-empty");
        let path = dir.join("build.gradle");
        write_file(&path, b"android {\n    compileSdkVersion 34\n}\n");
        assert_eq!(parse_application_id(&path), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_app_id_uses_key_properties_directory_flutter_layout() {
        // Flutter 실측 배치: <proj>/app/android/key.properties → <proj>/app/android/app/build.gradle.kts.
        // keystore_path 자체는 climbing 으로 안 빠지므로(key_properties_dir 히트) 무관한 값으로 둔다.
        let dir = temp_dir("find-app-id-flutter");
        let android_dir = dir.join("proj/app/android");
        write_file(&android_dir.join("app/build.gradle.kts"), b"applicationId = \"com.example.myapp\"\n");
        let irrelevant_keystore = dir.join("proj/app/android/release.jks");
        let app_id = find_app_id(&irrelevant_keystore, Some(&android_dir));
        assert_eq!(app_id, Some("com.example.myapp".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_app_id_climbs_from_keystore_when_key_properties_dir_absent() {
        // key.properties 를 못 찾은 경우(예: storeFile 불일치로 find_key_properties 가 None) — keystore
        // 경로에서 상위로 올라가며 <조상>/android/app/build.gradle 을 찾는다(모노레포 배치, 위
        // 실측과 동일 상대 구조: keystore 가 android/ 바로 밑에 있고 app/ 은 형제 폴더).
        let dir = temp_dir("find-app-id-climb");
        write_file(&dir.join("proj/apps/mobile/android/app/build.gradle"), b"applicationId 'com.example.otherapp'\n");
        let keystore_path = dir.join("proj/apps/mobile/android/release.jks");
        write_file(&keystore_path, b"dummy keystore bytes for test");
        assert_eq!(find_app_id(&keystore_path, None), Some("com.example.otherapp".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_app_id_none_when_no_gradle_project_nearby() {
        let dir = temp_dir("find-app-id-none");
        let keystore_path = dir.join("release.jks");
        write_file(&keystore_path, b"dummy keystore bytes for test");
        assert_eq!(find_app_id(&keystore_path, None), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_found_key_fills_app_id_end_to_end_via_key_properties() {
        // collect_if_candidate/build_found_key 전체 경로 — 더미 keystore + key.properties(storeFile 로
        // 서로 매칭) + build.gradle.kts 를 실제 파일로 준비해 FoundKeyKind::AndroidKeystore::app_id 까지
        // 채워지는지 확인한다(비밀번호는 필요 없다 — app_id 해석은 항상 시도된다).
        let dir = temp_dir("build-found-key-app-id");
        let keystore = dir.join("app/android/release.jks");
        write_file(&keystore, b"dummy keystore bytes for test");
        write_file(&dir.join("app/android/key.properties"), b"storeFile=release.jks\nkeyAlias=upload\n");
        write_file(&dir.join("app/android/app/build.gradle.kts"), b"applicationId = \"com.example.myapp\"\n");

        let metadata = fs::metadata(&keystore).unwrap();
        let found = build_found_key(&keystore, &metadata, "jks").expect("빌드 실패하면 안 된다");
        match found.kind {
            FoundKeyKind::AndroidKeystore { app_id, .. } => {
                assert_eq!(app_id, Some("com.example.myapp".to_string()));
            }
            other => panic!("AndroidKeystore 여야 한다: {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_app_id_fills_via_key_properties_fast_path() {
        // 실호출부(build_found_key) 경로로 fast path 만 검증하는 회귀 테스트 — key.properties 디렉터리를
        // "android" 가 아닌 이름(signing)으로 둬서 climbing(<조상>/android/app/build.gradle) 은 절대 못
        // 찾게 막고, applicationId 는 오직 key.properties 디렉터리 기준 app/build.gradle.kts(fast path)
        // 에만 존재하게 한다. build_found_key 가 find_app_id 에 key.properties "파일" 경로를 그대로
        // 넘기던 버그가 있으면(수정 전) fast path 가 항상 존재불가 경로를 만들어 못 찾고 climbing 도
        // 실패해 app_id 가 None 이 되어 이 테스트가 실패한다.
        let dir = temp_dir("fast-path-real-call");
        let signing_dir = dir.join("proj/signing");
        let keystore = signing_dir.join("release.jks");
        write_file(&keystore, b"dummy keystore bytes for test");
        write_file(&signing_dir.join("key.properties"), b"storeFile=release.jks\nkeyAlias=upload\n");
        write_file(&signing_dir.join("app/build.gradle.kts"), b"applicationId = \"com.example.fastpath\"\n");

        let metadata = fs::metadata(&keystore).unwrap();
        let found = build_found_key(&keystore, &metadata, "jks").expect("빌드 실패하면 안 된다");
        match found.kind {
            FoundKeyKind::AndroidKeystore { app_id, .. } => {
                assert_eq!(app_id, Some("com.example.fastpath".to_string()));
            }
            other => panic!("AndroidKeystore 여야 한다: {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_respects_depth_limit_per_root() {
        let home = temp_dir("depth-limit");
        // Projects 루트는 max_depth = 6 — "중간 디렉터리 6개"(a..f)까지는 찾고, 7개(a..g)는 못 찾아야
        // 한다(walk_dir 문서: 파일이 collect 되려면 중간 디렉터리 수 K 가 K <= max_depth 를 만족해야 함).
        let within = home.join("Projects/a/b/c/d/e/f/within.jks");
        let beyond = home.join("Projects/a/b/c/d/e/f/g/beyond.jks");
        write_file(&within, b"dummy");
        write_file(&beyond, b"dummy");
        let results = scan(&home);
        let paths: Vec<String> = results.iter().map(|f| f.path.clone()).collect();
        assert!(paths.contains(&within.to_string_lossy().to_string()), "중간 디렉터리 6개는 찾아야 한다");
        assert!(!paths.contains(&beyond.to_string_lossy().to_string()), "중간 디렉터리 7개는 못 찾아야 한다");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn scan_skips_named_directories() {
        let home = temp_dir("skip-dirs");
        let inside_node_modules = home.join("Projects/node_modules/pkg/hidden.jks");
        write_file(&inside_node_modules, b"dummy");
        let results = scan(&home);
        assert!(results.is_empty(), "node_modules 안은 절대 찾으면 안 된다");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn scan_does_not_follow_symlinks() {
        let home = temp_dir("symlink-home");
        // 실제 파일은 스캔 루트 완전히 바깥의 별도 temp dir 에 둔다 — home 안에 실제로 두면 (심링크와
        // 무관하게) 정상 순회로도 발견되어 이 테스트가 무엇을 검증하는지 흐려진다. 유일한 접근 경로가
        // 심링크뿐이어야 "심링크를 따라가면 찾아지고, 안 따라가면 안 찾아진다"를 제대로 확인할 수 있다.
        let outside = temp_dir("symlink-outside");
        let real_file = outside.join("secret.jks");
        write_file(&real_file, b"dummy");

        let linked_dir = home.join("Projects/linked");
        fs::create_dir_all(home.join("Projects")).unwrap();
        symlink(&outside, &linked_dir).expect("symlink 생성 실패하면 안 된다");

        let direct_link = home.join("Projects/direct-link.jks");
        symlink(&real_file, &direct_link).expect("symlink 생성 실패하면 안 된다");

        let results = scan(&home);
        assert!(results.is_empty(), "심링크(디렉터리·파일 모두)는 절대 따라가면 안 된다: {results:#?}");
        let _ = fs::remove_dir_all(&home);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn scan_excludes_debug_keystore_but_collects_release_ones() {
        let home = temp_dir("exclude-debug");
        write_file(&home.join("Projects/debug.keystore"), b"dummy-debug");
        write_file(&home.join("Projects/release.jks"), b"dummy-release");
        let results = scan(&home);
        let paths: Vec<String> = results.iter().map(|f| f.path.clone()).collect();
        assert!(!paths.iter().any(|p| p.ends_with("debug.keystore")), "debug.keystore 는 제외돼야 한다");
        assert!(paths.iter().any(|p| p.ends_with("release.jks")), "release.jks 는 포함돼야 한다");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn scan_dedupes_files_reachable_via_overlapping_roots() {
        let home = temp_dir("dedupe");
        // Projects/dup.jks 는 $HOME 루트(depth<=2)로도, Projects 전용 루트(depth<=6)로도 둘 다 닿는다.
        write_file(&home.join("Projects/dup.jks"), b"dummy");
        let results = scan(&home);
        let count = results.iter().filter(|f| f.path.ends_with("Projects/dup.jks")).count();
        assert_eq!(count, 1, "겹치는 루트로 두 번 발견돼도 결과는 한 번만 담겨야 한다");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn scan_detects_p8_subtypes_and_key_ids() {
        let home = temp_dir("p8-scan");
        write_file(&home.join("Downloads/AuthKey_ABC123DEFG.p8"), b"dummy-p8");
        write_file(&home.join("Downloads/SubscriptionKey_XYZ987.p8"), b"dummy-p8");
        let results = scan(&home);
        let auth = results
            .iter()
            .find(|f| f.path.ends_with("AuthKey_ABC123DEFG.p8"))
            .expect("AuthKey_*.p8 을 찾아야 한다");
        match &auth.kind {
            FoundKeyKind::AppleP8 { key_id, subtype } => {
                assert_eq!(key_id, "ABC123DEFG");
                assert_eq!(*subtype, P8Subtype::AppStoreConnectApi);
            }
            other => panic!("AndroidKeystore 가 아니라 AppleP8 이어야 한다: {other:?}"),
        }
        let sub = results
            .iter()
            .find(|f| f.path.ends_with("SubscriptionKey_XYZ987.p8"))
            .expect("SubscriptionKey_*.p8 을 찾아야 한다");
        match &sub.kind {
            FoundKeyKind::AppleP8 { key_id, subtype } => {
                assert_eq!(key_id, "XYZ987");
                assert_eq!(*subtype, P8Subtype::Subscription);
            }
            other => panic!("AndroidKeystore 가 아니라 AppleP8 이어야 한다: {other:?}"),
        }
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn import_android_signing_stores_password_in_keychain_when_available() {
        let Some(keytool) = crate::child_env::resolve_jdk_tool("keytool") else {
            return; // JDK 없는 환경 — signing.rs 의 e2e 테스트와 동일하게 건너뛴다.
        };
        let base_dir = temp_dir("import-with-pw-base");
        let keystore_dir = temp_dir("import-with-pw-keystore");
        let vault_dir = temp_dir("import-with-pw-vault");
        let keystore_path = keystore_dir.join("release.jks");
        let alias = format!("alias-{}", Uuid::new_v4());
        let store_pw = "storepw-test-123";

        let status = std::process::Command::new(&keytool)
            .args(["-genkeypair", "-storetype", "JKS", "-keystore"])
            .arg(&keystore_path)
            .args(["-storepass", store_pw, "-keypass", store_pw, "-alias", &alias])
            .args(["-dname", "CN=bildorak-keyscan-test", "-validity", "1", "-keyalg", "RSA", "-keysize", "2048"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("keytool 실행 자체가 실패하면 안 된다");
        if !status.success() || !keystore_path.is_file() {
            let _ = fs::remove_dir_all(&base_dir);
            let _ = fs::remove_dir_all(&keystore_dir);
            return; // 이 머신에서 keytool 생성이 안 되는 드문 환경 — 건너뛴다.
        }

        let properties_contents =
            format!("storeFile={}\nkeyAlias={alias}\nstorePassword={store_pw}\nkeyPassword={store_pw}\n", keystore_path.to_string_lossy());
        write_file(&keystore_dir.join("key.properties"), properties_contents.as_bytes());

        let result =
            import_android_signing(&base_dir, &vault_dir, &keystore_path, "project-1").expect("가져오기 실패하면 안 된다");
        assert!(result.imported, "비밀번호가 있으니 keychain 이관까지 성공해야 한다");
        assert_eq!(result.key_alias.as_deref(), Some(alias.as_str()));
        assert_eq!(result.key.linked_project_ids, vec!["project-1".to_string()]);
        let config = result.key.android_signing.clone().expect("androidSigning 이 채워져 있어야 한다");
        assert_eq!(
            signing::read_keychain_password(&config.store_password_service, &config.keychain_account).as_deref(),
            Ok(store_pw)
        );

        // 안전 보관 볼트 복사 — vault_path 가 채워지고, 그 경로에 원본과 같은 내용의 사본이 실제로
        // 있어야 하며, 원본은 그대로 남아 있어야 한다(이동 금지, 확정된 설계 결정).
        let vault_path = result.key.vault_path.clone().expect("Android keystore 는 vault_path 가 채워져야 한다");
        assert!(vault_path.starts_with(&vault_dir.to_string_lossy().to_string()), "볼트 폴더 밑에 있어야 한다");
        assert_eq!(fs::read(&vault_path).unwrap(), fs::read(&keystore_path).unwrap(), "볼트 사본 내용이 원본과 같아야 한다");
        assert!(keystore_path.is_file(), "원본 keystore 는 그대로 남아 있어야 한다");

        // 뒷정리 — keychain 항목과 임시 디렉터리를 남기지 않는다.
        signing::forget_android_signing_secrets(&config);
        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&keystore_dir);
        let _ = fs::remove_dir_all(&vault_dir);
    }

    #[test]
    fn import_android_signing_registers_without_keychain_when_password_missing() {
        let base_dir = temp_dir("import-no-pw-base");
        let keystore_dir = temp_dir("import-no-pw-keystore");
        let vault_dir = temp_dir("import-no-pw-vault");
        let keystore_path = keystore_dir.join("release.jks");
        write_file(&keystore_path, b"dummy keystore bytes for test");
        // key.properties 자체가 없다 — passwords_available 이 처음부터 false 인 시나리오.

        let result =
            import_android_signing(&base_dir, &vault_dir, &keystore_path, "project-1").expect("가져오기 실패하면 안 된다");
        assert!(!result.imported, "비밀번호를 못 찾았으니 keychain 이관은 안 돼야 한다");
        assert!(result.key.android_signing.is_none());
        assert_eq!(result.key.linked_project_ids, vec!["project-1".to_string()]);
        // 비밀번호를 못 찾아도 볼트 백업은 별개로 진행돼야 한다(안전 보관은 비밀번호 유무와 무관).
        assert!(result.key.vault_path.is_some(), "비밀번호가 없어도 볼트 복사는 돼야 한다");

        // signing_keys.json 에는 등록·연결까지는 반영돼 있어야 한다(다음에 또 스캔 안 해도 되게).
        let saved = signing::load_signing_keys(&base_dir).expect("읽기 실패하면 안 된다");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, result.key.id);

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&keystore_dir);
        let _ = fs::remove_dir_all(&vault_dir);
    }

    // ── 프로젝트 자체 key.properties 자동 채움 e2e(autofill_android_signing_from_project) ────────

    #[test]
    fn autofill_android_signing_from_project_matches_and_stores_in_keychain() {
        let Some(keytool) = crate::child_env::resolve_jdk_tool("keytool") else {
            return; // JDK 없는 환경 — 다른 keytool e2e 테스트와 동일하게 건너뛴다.
        };
        let base_dir = temp_dir("autofill-match-base");
        let repo_dir = temp_dir("autofill-match-repo");
        let keystore_dir = temp_dir("autofill-match-keystore"); // 프로젝트 폴더 밖 — "홑파일" 시나리오.
        let keystore_path = keystore_dir.join("release.jks");
        let alias = format!("alias-{}", Uuid::new_v4());
        let store_pw = "storepw-autofill-test";

        let status = std::process::Command::new(&keytool)
            .args(["-genkeypair", "-storetype", "JKS", "-keystore"])
            .arg(&keystore_path)
            .args(["-storepass", store_pw, "-keypass", store_pw, "-alias", &alias])
            .args(["-dname", "CN=bildorak-autofill-test", "-validity", "1", "-keyalg", "RSA", "-keysize", "2048"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("keytool 실행 자체가 실패하면 안 된다");
        if !status.success() || !keystore_path.is_file() {
            let _ = fs::remove_dir_all(&base_dir);
            let _ = fs::remove_dir_all(&repo_dir);
            let _ = fs::remove_dir_all(&keystore_dir);
            return; // 이 머신에서 keytool 생성이 안 되는 드문 환경 — 건너뛴다.
        }

        // 프로젝트 자체 key.properties — repo_path/android/key.properties, 홑파일 keystore(별도 폴더)를
        // 절대경로로 가리킨다(find_project_key_properties 문서의 경로 규칙 그대로).
        let properties_contents = format!(
            "storeFile={}\nkeyAlias={alias}\nstorePassword={store_pw}\nkeyPassword={store_pw}\n",
            keystore_path.to_string_lossy()
        );
        write_file(&repo_dir.join("android/key.properties"), properties_contents.as_bytes());

        // register_signing_key 가 이미 등록해 둔 상태를 흉내낸다(build_record 로 레코드를 만들고 저장) —
        // 이 함수는 새 레코드를 만들지 않고 기존 레코드만 갱신한다(파일 상단 문서 참고).
        let mut record = signing::build_record(&keystore_path).expect("등록 실패하면 안 된다");
        record.linked_project_ids.push("project-1".to_string());
        signing::save_signing_keys(&base_dir, &[record.clone()]).expect("저장 실패하면 안 된다");

        let result = autofill_android_signing_from_project(&base_dir, &repo_dir, &record.id)
            .expect("자동 채움 실패하면 안 된다");
        assert!(result.imported, "storeFile 이 일치하고 비밀번호가 있으니 자동 이관돼야 한다");
        assert_eq!(result.key_alias.as_deref(), Some(alias.as_str()));
        let config = result.key.android_signing.clone().expect("androidSigning 이 채워져야 한다");
        assert_eq!(
            signing::read_keychain_password(&config.store_password_service, &config.keychain_account).as_deref(),
            Ok(store_pw)
        );

        signing::forget_android_signing_secrets(&config);
        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
        let _ = fs::remove_dir_all(&keystore_dir);
    }

    #[test]
    fn autofill_android_signing_from_project_falls_back_when_store_file_mismatches() {
        let base_dir = temp_dir("autofill-mismatch-base");
        let repo_dir = temp_dir("autofill-mismatch-repo");
        let keystore_dir = temp_dir("autofill-mismatch-keystore");
        let keystore_path = keystore_dir.join("release.jks");
        write_file(&keystore_path, b"dummy keystore bytes for test");

        // 프로젝트 key.properties 는 완전히 다른 keystore 를 가리킨다 — 안전 매칭 실패 시나리오
        // (엉뚱한 비밀번호를 잘못 연결하지 않는지 확인).
        let other_keystore = keystore_dir.join("other.jks");
        write_file(&other_keystore, b"dummy-other");
        let properties_contents = format!(
            "storeFile={}\nkeyAlias=upload\nstorePassword=storepw\nkeyPassword=keypw\n",
            other_keystore.to_string_lossy()
        );
        write_file(&repo_dir.join("android/key.properties"), properties_contents.as_bytes());

        let mut record = signing::build_record(&keystore_path).expect("등록 실패하면 안 된다");
        record.linked_project_ids.push("project-1".to_string());
        signing::save_signing_keys(&base_dir, &[record.clone()]).expect("저장 실패하면 안 된다");

        let result = autofill_android_signing_from_project(&base_dir, &repo_dir, &record.id)
            .expect("불일치여도 함수 자체는 실패하면 안 된다(수동 폴백 신호만 돌려준다)");
        assert!(!result.imported, "storeFile 불일치는 자동 채움을 시도하면 안 된다");
        assert!(result.key.android_signing.is_none());

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
        let _ = fs::remove_dir_all(&keystore_dir);
    }

    #[test]
    fn autofill_android_signing_from_project_falls_back_when_original_keystore_is_stale() {
        // stale 원본 시나리오(리뷰 지적, 리뷰에서 강조된 "엉뚱한 비번 방지") — 홑파일 keystore 등록 후
        // 원본을 이동/삭제하면 record.file_path 는 그대로 남는다(stale). 이 상태에서 프로젝트
        // key.properties 가 "다른" keystore(other_keystore, 여전히 실존)를 storeFile 로 가리키면, 수정
        // 전에는 store_file_matches 의 keystore_path.canonicalize() 실패가 관대한 `_ => true` 로 빠져
        // other_keystore 의 비밀번호가 이 레코드 keychain 에 잘못 이관됐다. 수정 후에는 원본 실재 가드가
        // 먼저 걸려 매칭 자체를 시도하지 않는다 — imported:false 로 폴백해야 한다.
        let base_dir = temp_dir("autofill-stale-original-base");
        let repo_dir = temp_dir("autofill-stale-original-repo");
        let keystore_dir = temp_dir("autofill-stale-original-keystore");
        let keystore_path = keystore_dir.join("release.jks"); // 등록 당시 경로 — 아래에서 삭제해 stale 화.
        write_file(&keystore_path, b"dummy keystore bytes for test");

        let mut record = signing::build_record(&keystore_path).expect("등록 실패하면 안 된다");
        record.linked_project_ids.push("project-1".to_string());
        signing::save_signing_keys(&base_dir, &[record.clone()]).expect("저장 실패하면 안 된다");

        // 원본을 "이동/삭제" — 레코드의 file_path 는 여전히 이 경로를 가리키지만 실제 파일은 없다.
        fs::remove_file(&keystore_path).expect("삭제 실패하면 안 된다");

        // 프로젝트 key.properties 는 완전히 "다른" keystore 를 가리킨다 — 그 앱에 이미 별개의 keystore 가
        // 연결돼 있는 상태를 흉내낸다.
        let other_keystore = keystore_dir.join("other.jks");
        write_file(&other_keystore, b"dummy-other");
        let properties_contents = format!(
            "storeFile={}\nkeyAlias=upload\nstorePassword=storepw\nkeyPassword=keypw\n",
            other_keystore.to_string_lossy()
        );
        write_file(&repo_dir.join("android/key.properties"), properties_contents.as_bytes());

        let result = autofill_android_signing_from_project(&base_dir, &repo_dir, &record.id)
            .expect("원본이 사라져도 함수 자체는 실패하면 안 된다(수동 폴백 신호만 돌려준다)");
        // 버그가 있으면 여기서 other_keystore 의 비밀번호가 실제로 keychain 에 저장된다 — assert 전에
        // 먼저 치운다(테스트가 실패하더라도 더미 keychain 항목을 남기지 않는다).
        if let Some(config) = result.key.android_signing.clone() {
            signing::forget_android_signing_secrets(&config);
        }
        assert!(
            !result.imported,
            "원본 keystore 가 사라졌으면(stale) 엉뚱한 매칭을 하면 안 된다 — imported:false 로 폴백해야 한다"
        );
        assert!(result.key.android_signing.is_none(), "엉뚱한 keystore 의 비밀번호가 keychain 에 이관되면 안 된다");

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
        let _ = fs::remove_dir_all(&keystore_dir);
    }

    #[test]
    fn autofill_android_signing_from_project_no_op_when_project_has_no_key_properties() {
        let base_dir = temp_dir("autofill-none-base");
        let repo_dir = temp_dir("autofill-none-repo");
        let keystore_dir = temp_dir("autofill-none-keystore");
        let keystore_path = keystore_dir.join("release.jks");
        write_file(&keystore_path, b"dummy keystore bytes for test");
        // repo_dir/android/key.properties 자체가 없다.

        let mut record = signing::build_record(&keystore_path).expect("등록 실패하면 안 된다");
        record.linked_project_ids.push("project-1".to_string());
        signing::save_signing_keys(&base_dir, &[record.clone()]).expect("저장 실패하면 안 된다");

        let result = autofill_android_signing_from_project(&base_dir, &repo_dir, &record.id)
            .expect("key.properties 가 없어도 함수 자체는 실패하면 안 된다");
        assert!(!result.imported);
        assert!(result.key_alias.is_none());

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
        let _ = fs::remove_dir_all(&keystore_dir);
    }

    #[test]
    fn autofill_android_signing_from_project_rejects_non_android_kind() {
        let base_dir = temp_dir("autofill-wrong-kind-base");
        let repo_dir = temp_dir("autofill-wrong-kind-repo");
        let cert_dir = temp_dir("autofill-wrong-kind-cert");
        let cert_path = cert_dir.join("AuthKey_ABC123.p8");
        write_file(&cert_path, b"-----BEGIN PRIVATE KEY-----\ndummy\n-----END PRIVATE KEY-----\n");

        let record = signing::build_record(&cert_path).expect("등록 실패하면 안 된다");
        assert_eq!(record.kind, crate::model::SigningKeyKind::IosApiKey);
        signing::save_signing_keys(&base_dir, &[record.clone()]).expect("저장 실패하면 안 된다");

        let result = autofill_android_signing_from_project(&base_dir, &repo_dir, &record.id);
        assert!(result.is_err(), "Android keystore 가 아니면 에러여야 한다");

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
        let _ = fs::remove_dir_all(&cert_dir);
    }

    #[test]
    fn autofill_android_signing_from_project_missing_key_id_is_error() {
        let base_dir = temp_dir("autofill-missing-key-base");
        let repo_dir = temp_dir("autofill-missing-key-repo");
        let result = autofill_android_signing_from_project(&base_dir, &repo_dir, "does-not-exist");
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    #[test]
    fn register_found_store_key_is_idempotent_by_path() {
        let base_dir = temp_dir("store-key-idempotent");
        let first = register_found_store_key(&base_dir, "/tmp/AuthKey_ABC123.p8", "ABC123", P8Subtype::AppStoreConnectApi)
            .expect("첫 등록은 성공해야 한다");
        let second = register_found_store_key(&base_dir, "/tmp/AuthKey_ABC123.p8", "ABC123", P8Subtype::AppStoreConnectApi)
            .expect("두 번째 호출도 실패하면 안 된다");
        assert_eq!(first.id, second.id, "같은 경로는 같은 레코드를 돌려줘야 한다(멱등)");
        let all = load_found_store_keys(&base_dir).expect("읽기 실패하면 안 된다");
        assert_eq!(all.len(), 1, "중복 레코드가 쌓이면 안 된다");
        let _ = fs::remove_dir_all(&base_dir);
    }
}
