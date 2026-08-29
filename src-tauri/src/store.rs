// store.rs — projects.json 읽기/쓰기(Tauri app config dir). 등록/해제 시 배열 전체를 다시 쓴다
// (프로젝트 수가 적은 개인 데스크톱 앱이라 부분 갱신 대신 단순 read-modify-write로 충분).

use crate::model::ProjectRecord;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

const PROJECTS_FILE: &str = "projects.json";
const KEYSTORE_VAULT_DIR_NAME: &str = "keystores";

/// 이 앱의 설정/상태 저장 폴더(app config dir) — 없으면 만든다. projects.json 뿐 아니라 build.rs 의
/// 빌드 job/로그도 이 폴더 하위에 둔다(commands.rs 가 build 커맨드에서 이 함수로 base_dir 을 구한다) —
/// "어디에 앱 자체 데이터를 두는지"를 한 곳에서만 결정해 store.rs/build.rs 가 서로 어긋나지 않게 한다.
pub(crate) fn app_config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("설정 폴더 경로를 확인하지 못했어요: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("설정 폴더를 만들지 못했어요: {e}"))?;
    Ok(dir)
}

fn projects_file_path_from_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(PROJECTS_FILE)
}

/// keystore_vault_dir(app) 의 실제 계산(base_dir 만 있으면 됨) — CLI(bildorak-cli) 전용 진입점으로
/// additive 추출했다. macOS 에서는 app_data_dir 과 app_config_dir 이 같은 물리 경로로 귀결되므로(위
/// keystore_vault_dir 문서 참고), paths::base_dir() 을 그대로 넘겨도 GUI 와 같은 폴더를 가리킨다.
pub fn keystore_vault_dir_from_dir(base_dir: &Path) -> Result<PathBuf, String> {
    let dir = base_dir.join(KEYSTORE_VAULT_DIR_NAME);
    fs::create_dir_all(&dir).map_err(|e| format!("keystore 보관 폴더를 만들지 못했어요: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("keystore 보관 폴더 권한을 설정하지 못했어요: {e}"))?;
    }
    Ok(dir)
}

/// keystore 안전 보관 볼트 폴더(app_data_dir 하위 "keystores/") — 없으면 만들고 소유자 전용 권한(0700)을
/// 준다(확정된 설계 결정, signing.rs::copy_keystore_into_vault 문서 참고). app_config_dir 과는 별도
/// base(app_data_dir)를 쓴다 — macOS 에선 실측상 같은 물리 경로(~/Library/Application Support/<bundle>)로
/// 귀결되기도 하지만, 하드코딩 대신 의미상 맞는 Tauri path API 를 쓴다(설계 결정). 권한 설정은 Unix
/// 전용(cfg(unix)) — 이 앱은 지금 macOS 만 지원하지만 향후 Windows 확장 시에도
/// 여기서 컴파일이 깨지지 않게 방어적으로 가드한다.
pub(crate) fn keystore_vault_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("데이터 폴더 경로를 확인하지 못했어요: {e}"))?;
    keystore_vault_dir_from_dir(&base)
}

/// load_projects(app) 의 실제 읽기 로직(base_dir 만 있으면 됨) — CLI(bildorak-cli) 전용 진입점으로
/// additive 추출했다(settings.rs::load_settings_from_dir 과 동일 패턴). 파일이 없으면(첫 실행) 빈 목록.
pub fn load_projects_from_dir(base_dir: &Path) -> Result<Vec<ProjectRecord>, String> {
    let path = projects_file_path_from_dir(base_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("등록된 프로젝트 목록을 읽지 못했어요: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&raw).map_err(|e| format!("등록된 프로젝트 목록이 손상됐어요: {e}"))
}

/// 저장된 목록을 읽는다. 파일이 없으면(첫 실행) 빈 목록.
pub fn load_projects(app: &AppHandle) -> Result<Vec<ProjectRecord>, String> {
    load_projects_from_dir(&app_config_dir(app)?)
}

/// save_projects(app) 의 실제 쓰기 로직(base_dir 만 있으면 됨) — CLI(bildorak-cli) 전용 진입점으로
/// additive 추출했다(pretty JSON — 사람이 열어봐도 읽히게).
pub fn save_projects_to_dir(base_dir: &Path, projects: &[ProjectRecord]) -> Result<(), String> {
    let path = projects_file_path_from_dir(base_dir);
    let raw = serde_json::to_string_pretty(projects)
        .map_err(|e| format!("저장할 데이터를 만들지 못했어요: {e}"))?;
    write_json_atomic(&path, &raw).map_err(|e| format!("프로젝트 목록을 저장하지 못했어요: {e}"))
}

/// 목록 전체를 저장한다(pretty JSON — 사람이 열어봐도 읽히게).
pub fn save_projects(app: &AppHandle, projects: &[ProjectRecord]) -> Result<(), String> {
    save_projects_to_dir(&app_config_dir(app)?, projects)
}

/// temp 파일에 쓰고 rename 으로 교체하는 원자적 저장 — 저장 도중 앱이 죽거나 강제 종료돼도
/// 기존 파일이 반쯤 쓰인 내용으로 손상되지 않는다(rename 은 같은 파일시스템 내에서 원자적).
/// build.rs 의 build_jobs.json 저장도 이 함수를 그대로 재사용한다(설계 요구사항).
pub(crate) fn write_json_atomic(path: &Path, raw: &str) -> std::io::Result<()> {
    let tmp_path = PathBuf::from(format!("{}.tmp", path.to_string_lossy()));
    fs::write(&tmp_path, raw)?;
    fs::rename(&tmp_path, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Platform;

    fn temp_base_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bildorak-store-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("failed to create temp base dir");
        dir
    }

    fn fake_project() -> ProjectRecord {
        ProjectRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name: "테스트 프로젝트".to_string(),
            selected_path: "/tmp/selected".to_string(),
            repo_path: "/tmp/repo".to_string(),
            version: Some("1.0.0".to_string()),
            build_number: Some("1".to_string()),
            platforms: vec![Platform::Android],
            registered_at: "2026-01-01T00:00:00+00:00".to_string(),
        }
    }

    /// CLI(paths::base_dir()) 와 GUI(app_config_dir(app)) 가 같은 폴더를 base_dir 로 넘기면 완전히
    /// 같은 데이터를 주고받아야 한다 — *_from_dir 추출이 기존 read-modify-write 규칙을 그대로
    /// 유지하는지 확인한다(무회귀).
    #[test]
    fn load_projects_from_dir_round_trips_with_save() {
        let dir = temp_base_dir();
        assert!(load_projects_from_dir(&dir).expect("빈 목록이어야 한다").is_empty());

        let project = fake_project();
        save_projects_to_dir(&dir, std::slice::from_ref(&project)).expect("저장 실패하면 안 된다");

        let loaded = load_projects_from_dir(&dir).expect("읽기 실패하면 안 된다");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, project.id);
        assert_eq!(loaded[0].name, project.name);

        let _ = fs::remove_dir_all(&dir);
    }

    /// keystore_vault_dir_from_dir 은 폴더를 만들고(0700, unix) base_dir 하위 "keystores/" 를 가리켜야
    /// 한다 — keystore_vault_dir(app) 이 감싸는 실제 로직과 동일 규칙.
    #[test]
    fn keystore_vault_dir_from_dir_creates_keystores_subfolder() {
        let dir = temp_base_dir();
        let vault = keystore_vault_dir_from_dir(&dir).expect("생성 실패하면 안 된다");
        assert_eq!(vault, dir.join(KEYSTORE_VAULT_DIR_NAME));
        assert!(vault.is_dir());
        let _ = fs::remove_dir_all(&dir);
    }
}
