// commands.rs — 프론트(React)가 invoke() 로 부르는 Tauri 커맨드 전체.
// 하드 제약(엔진 원칙): 프론트에서 실행 명령 문자열을 받지 않는다. 여기서 받는 파라미터는
// (1) 네이티브 다이얼로그가 돌려준 폴더 경로(등록 시점, 사용자가 OS 창에서 직접 고른 값) 또는
// (2) 이미 등록된 project_id 뿐이다 — 실제 실행 경로/argv 는 항상 이 파일 뒤(store/preflight)의
// 서버측 고정 로직이 결정한다.

use crate::build;
use crate::key_scan;
use crate::model::{
    AppSettings, BuildJob, BuildJobStatus, BuildStatus, BuildTarget, CliCommandDoc, FoundKey,
    FoundStoreKeyRecord, ImportAndroidSigningResult, KeySourceInfo, P8Subtype, PreflightRun, ProjectRecord,
    SigningKeyKind, SigningKeyRecord,
};
use crate::preflight;
use crate::pubspec;
use crate::settings;
use crate::signing;
use crate::store;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;

/// pick_project_folder 가 고른 실제 경로를 register_project 가 쓸 때까지 Rust 프로세스 메모리에만
/// 보관한다 — webview 로 원본 경로 문자열이 왕복하지 않게 한다(설계 요구사항 — "표면
/// 축소"). 토큰은 1회용 — register_project 가 소비하면 즉시 제거한다.
static PICKED_FOLDERS: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

fn picked_folders() -> &'static Mutex<HashMap<String, PathBuf>> {
    PICKED_FOLDERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// pick_signing_key_file 이 고른 실제 파일 경로를 register_signing_key 가 쓸 때까지 보관한다 —
/// picked_folders()/PICKED_FOLDERS 와 같은 목적(설계 요구사항 — "표면 축소")이지만 폴더
/// 선택(프로젝트 등록)과 완전히 별개 흐름이라 맵을 따로 둔다. 서명키는 특히 민감한 파일이라 이
/// 원칙을 그대로 지킨다 — webview 로 실제 경로 문자열이 왕복하지 않는다.
static PICKED_SIGNING_FILES: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

fn picked_signing_files() -> &'static Mutex<HashMap<String, PathBuf>> {
    PICKED_SIGNING_FILES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// project_id 로 등록된 프로젝트 하나를 찾는다 — start_build/get_build_status 양쪽이 같은 조회를
/// 반복하므로 한 곳으로 모은다.
fn find_project(app: &AppHandle, project_id: &str) -> Result<ProjectRecord, String> {
    store::load_projects(app)?
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| "등록된 프로젝트를 찾지 못했어요.".to_string())
}

/// 서명키 관리 커맨드 공통 — base_dir 를 돌려준다. 서명키 "관리"(등록·보기·연결·만료 확인)와 실제
/// "서명 + 스토어 업로드"까지 전부 무료다(전부 로컬·비용 0, 무료 오픈소스라 게이트 없음).
fn signing_base_dir(app: &AppHandle) -> Result<PathBuf, String> {
    store::app_config_dir(app)
}

/// 네이티브 "폴더 선택" 다이얼로그를 띄운다. 사용자가 취소하면 None. 반환값은 실제 경로 문자열이
/// 아니라 picked_folders() 에 보관해 둔 결과를 가리키는 1회용 토큰이다 — register_project 가 이
/// 토큰으로 실제 경로를 다시 찾는다(설계 요구사항 — webview 왕복 경로 문자열 표면 축소).
/// blocking_pick_folder() 는 메인 스레드에서 부르면 안 되므로 spawn_blocking 전용 스레드에서 실행.
#[tauri::command]
pub async fn pick_project_folder(app: AppHandle) -> Result<Option<String>, String> {
    let picked_path = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .map(|folder| folder.into_path())
    })
    .await
    .map_err(|e| format!("폴더 선택 창을 여는 중 문제가 발생했어요: {e}"))?
    .transpose()
    .map_err(|e| format!("선택한 폴더 경로를 확인하지 못했어요: {e}"))?;

    let Some(path) = picked_path else {
        return Ok(None);
    };
    let token = uuid::Uuid::new_v4().to_string();
    picked_folders()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(token.clone(), path);
    Ok(Some(token))
}

/// 고른 폴더 토큰(pick_project_folder 가 돌려준 값)으로 실제 경로를 찾아 pubspec.yaml 을 읽고 등록
/// 목록에 저장한다. 토큰은 여기서 소비된다(1회용) — 등록이 실패해도 같은 토큰을 재사용하지 않고
/// 프론트가 폴더 선택부터 다시 하게 한다(기존 흐름과 동일: "추가" 버튼을 다시 누르면 새로 고른다).
#[tauri::command]
pub async fn register_project(app: AppHandle, folder_token: String) -> Result<ProjectRecord, String> {
    let selected = picked_folders()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&folder_token)
        .ok_or_else(|| "선택한 폴더 정보를 찾지 못했어요. 폴더를 다시 선택해 주세요.".to_string())?;

    let detected = pubspec::detect_project(&selected)?;
    let repo_path_str = detected.repo_path.to_string_lossy().to_string();

    let mut projects = store::load_projects(&app)?;
    if projects.iter().any(|p| p.repo_path == repo_path_str) {
        return Err("이미 등록된 프로젝트예요.".to_string());
    }

    let record = ProjectRecord {
        id: uuid::Uuid::new_v4().to_string(),
        name: detected.name,
        selected_path: selected.to_string_lossy().to_string(),
        repo_path: repo_path_str,
        version: detected.version,
        build_number: detected.build_number,
        platforms: detected.platforms,
        registered_at: chrono::Utc::now().to_rfc3339(),
    };

    projects.push(record.clone());
    store::save_projects(&app, &projects)?;
    Ok(record)
}

/// 등록된 프로젝트 전체 목록.
#[tauri::command]
pub async fn list_projects(app: AppHandle) -> Result<Vec<ProjectRecord>, String> {
    store::load_projects(&app)
}

/// 등록 해제 — 프로젝트 폴더 자체는 건드리지 않고 등록 목록에서만 제거한다.
#[tauri::command]
pub async fn remove_project(app: AppHandle, project_id: String) -> Result<(), String> {
    let mut projects = store::load_projects(&app)?;
    let before = projects.len();
    projects.retain(|p| p.id != project_id);
    if projects.len() == before {
        return Err("등록된 프로젝트를 찾지 못했어요.".to_string());
    }
    store::save_projects(&app, &projects)
}

/// 빌드 준비 점검 실행 — project_id 만 받는다. 실제 repoPath/명령은 등록된 값만 사용(엔진 원칙).
/// 외부 프로세스를 여러 번 띄우고 최대 20초씩 기다릴 수 있어 spawn_blocking 으로 실행한다.
#[tauri::command]
pub async fn run_preflight(app: AppHandle, project_id: String) -> Result<PreflightRun, String> {
    let project = find_project(&app, &project_id)?;

    tauri::async_runtime::spawn_blocking(move || preflight::run(&project))
        .await
        .map_err(|e| format!("점검을 완료하지 못했어요: {e}"))
}

/// 빌드 완료 알림(2단계, 2026-08-16 전 사용자 무료 전환) — start_build 가
/// job(running) 을 돌려준 직후에만 호출된다. build.rs 는 AppHandle 을 모르는 경계를 유지하므로
/// (build.rs 파일 상단 주석) 완료 감지 자체를 그 내부 스레드에 맡기지 않고, 여기서 get_build_status 를
/// 짧은 간격으로 다시 물어보다가 이 job_id 가 더 이상 running 이 아니면(=finalize_build_job 이 이미
/// 기록을 마쳤으면) 알림을 띄운다.
fn spawn_build_finish_notifier(app: AppHandle, base_dir: PathBuf, project: ProjectRecord, job_id: String) {
    std::thread::spawn(move || {
        // build.rs 의 BUILD_TIMEOUT(45분)보다 넉넉히 길게 잡아 정상 종료는 전부 이 안에서 잡히게 하고,
        // 그래도 못 잡으면(상태 파일 접근 문제 지속 등) 이 백그라운드 스레드가 좀비처럼 영영 돌지 않게
        // 한다(build.rs 의 BUILD_TIMEOUT 안전망과 같은 목적).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(50 * 60);
        loop {
            if std::time::Instant::now() > deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1000));
            let Ok(status) = build::get_build_status(&base_dir, &project) else {
                continue;
            };
            let Some(job) = status.job else {
                break;
            };
            if job.id != job_id {
                break; // 우리가 지켜보던 job 이 다른 job 으로 대체됐다 — 더 기다릴 이유 없음(조용히 종료).
            }
            if job.status == BuildJobStatus::Running {
                continue;
            }
            let result_label = if job.status == BuildJobStatus::Success { "성공" } else { "실패" };
            let body = format!("{} — {}", project.name, result_label);
            let _ = app.notification().builder().title("빌드 완료").body(body).show();
            break;
        }
    });
}

/// 로컬 빌드 실행(2차) — project_id + target(enum) 만 받는다. 실제 bin/args 는 build::resolve_command
/// 의 고정 맵에서만 나온다(엔진 원칙, 파일 상단 주석과 동일). spawn 자체는 즉시 끝나고(child.wait() 는
/// build.rs 내부 스레드가 백그라운드로 처리) job(running) 을 바로 돌려주므로 spawn_blocking 은 쓰지
/// 않는다(register_project 등 다른 빠른 IO 커맨드와 같은 관례).
///
/// release 타겟(IosRelease/AndroidRelease, 1차)도 디버그 타겟과 동일하게 게이트 없이 무료다
/// (오픈소스 전환으로 앱 개수 한도까지 포함해 유료/라이선스 게이트 전체를 제거했다).
#[tauri::command]
pub async fn start_build(
    app: AppHandle,
    project_id: String,
    target: BuildTarget,
) -> Result<BuildJob, String> {
    let project = find_project(&app, &project_id)?;
    let base_dir = store::app_config_dir(&app)?;
    let job = build::start_build(&base_dir, &project, target)?;
    // 실제로 백그라운드에서 도는 빌드이고, 설정 화면에서 빌드 완료 알림을 켜 뒀을 때만(기본값 켬,
    // settings.rs::load_settings 문서 참고 — 읽기 실패해도 기존 동작대로 켬으로 물러나 무회귀) 완료
    // 알림 스레드를 띄운다. spawn 자체가 즉시 실패한 경우(job.status == Failed)는 이미 화면에 에러가
    // 보이므로 알림까지 중복으로 띄우지 않는다.
    if job.status == BuildJobStatus::Running {
        let notifications_enabled = settings::load_settings(&app)
            .map(|s| s.build_notifications_enabled)
            .unwrap_or(true);
        if notifications_enabled {
            spawn_build_finish_notifier(app, base_dir, project, job.id.clone());
        }
    }
    Ok(job)
}

/// 빌드 상태 조회(2차) — 앱 진입 시 마지막 결과 복원 + 진행 중일 때 프론트 폴링 양쪽에 쓰인다.
#[tauri::command]
pub async fn get_build_status(app: AppHandle, project_id: String) -> Result<BuildStatus, String> {
    let project = find_project(&app, &project_id)?;
    let base_dir = store::app_config_dir(&app)?;
    build::get_build_status(&base_dir, &project)
}

/// 진행 중인 빌드를 취소한다(설계 요구사항) — project_id 만 받는다. 실제 kill 대상 pid 는 항상
/// 서버측(Rust) 이 추적하는 값만 쓴다(엔진 원칙 — 프론트가 pid 를 직접 넘기지 않는다).
#[tauri::command]
pub async fn cancel_build(app: AppHandle, project_id: String) -> Result<(), String> {
    let project = find_project(&app, &project_id)?;
    let base_dir = store::app_config_dir(&app)?;
    build::cancel_build(&base_dir, &project)
}

/// 빌드 히스토리 조회(2단계, 2026-08-16 전 사용자 무료 전환) — project_id 만 받는다.
#[tauri::command]
pub async fn get_build_history(app: AppHandle, project_id: String) -> Result<Vec<BuildJob>, String> {
    let base_dir = store::app_config_dir(&app)?;
    build::get_build_history(&base_dir, &project_id)
}

// ── 서명키 관리(출시 준비 1차 골격, 무료 오픈소스) ─────────────────────────────────
// 실제 등록/파싱 로직은 signing.rs 가 담당한다 — 여기는 파일 토큰 관리 + 프로젝트
// 존재 확인만 한다(엔진 원칙: 프론트는 파일 토큰/project_id/key_id 만 넘긴다).

/// 네이티브 "파일 선택" 다이얼로그(서명키 등록용) — pick_project_folder 와 동일한 토큰 패턴(위
/// PICKED_SIGNING_FILES 주석 참고). 인증서/키 확장자로 필터를 걸어 두지만, 실제 종류 판정은 항상
/// signing.rs::detect_kind 가 다시 한다(다이얼로그 필터는 사용자 편의일 뿐 신뢰 경계가 아니다).
#[tauri::command]
pub async fn pick_signing_key_file(app: AppHandle) -> Result<Option<String>, String> {
    let picked_path = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("서명키 파일", &["cer", "pem", "crt", "p12", "p8", "jks", "keystore"])
            .blocking_pick_file()
            .map(|file| file.into_path())
    })
    .await
    .map_err(|e| format!("파일 선택 창을 여는 중 문제가 발생했어요: {e}"))?
    .transpose()
    .map_err(|e| format!("선택한 파일 경로를 확인하지 못했어요: {e}"))?;

    let Some(path) = picked_path else {
        return Ok(None);
    };
    let token = uuid::Uuid::new_v4().to_string();
    picked_signing_files()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(token.clone(), path);
    Ok(Some(token))
}

/// 등록된 서명키 전체 목록(무료 오픈소스, "로컬 편의" 성격).
#[tauri::command]
pub async fn list_signing_keys(app: AppHandle) -> Result<Vec<SigningKeyRecord>, String> {
    let base_dir = signing_base_dir(&app)?;
    signing::load_signing_keys(&base_dir)
}

/// 고른 파일 토큰(pick_signing_key_file 이 돌려준 값)으로 실제 경로를 찾아 종류 감지 + 메타데이터
/// 추출 후 등록 목록에 저장한다. 토큰은 여기서 소비된다(1회용, register_project 와 동일 규칙).
/// signing::build_record 가 iOS 인증서일 때 openssl 을 실행할 수 있어 run_preflight 와 동일하게
/// spawn_blocking 으로 async 런타임 스레드를 막지 않는다.
#[tauri::command]
pub async fn register_signing_key(app: AppHandle, file_token: String) -> Result<SigningKeyRecord, String> {
    let base_dir = signing_base_dir(&app)?;
    let selected = picked_signing_files()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&file_token)
        .ok_or_else(|| "선택한 파일 정보를 찾지 못했어요. 파일을 다시 선택해 주세요.".to_string())?;

    let join_result = tauri::async_runtime::spawn_blocking(move || signing::build_record(&selected))
        .await
        .map_err(|e| format!("서명키 정보를 확인하는 중 문제가 발생했어요: {e}"))?;
    let mut record = join_result?;

    // Android keystore 는 원본을 안전 보관 볼트로 복사해 둔다(분실 대비 백업, 확정된 설계 결정) — 원본은
    // 절대 옮기거나 고치지 않는다(signing::copy_keystore_into_vault 문서 참고). iOS 인증서/API 키는
    // 볼트 대상이 아니다(범위 밖 — signing.rs 파일 상단 주석 그대로 참조만 한다).
    if record.kind == SigningKeyKind::AndroidKeystore {
        let vault_dir = store::keystore_vault_dir(&app)?;
        let original_path = PathBuf::from(&record.file_path);
        let key_id = record.id.clone();
        let vault_join = tauri::async_runtime::spawn_blocking(move || {
            signing::copy_keystore_into_vault(&vault_dir, &original_path, &key_id)
        })
        .await
        .map_err(|e| format!("keystore를 안전 보관 폴더에 복사하는 중 문제가 발생했어요: {e}"))?;
        record.vault_path = Some(vault_join?.to_string_lossy().to_string());
    }

    let mut keys = signing::load_signing_keys(&base_dir)?;
    keys.push(record.clone());
    signing::save_signing_keys(&base_dir, &keys)?;
    Ok(record)
}

/// 등록 해제(완전 삭제) — signing_keys.json 목록에서만 제거한다. 원본 키 파일은 애초에 복사한 적이
/// 없으니 건드릴 것도 없다(보안 원칙, signing.rs 파일 상단 주석). 여러 프로젝트에 연결돼 있었다면
/// 전부에서 함께 사라진다(linked_project_ids 는 이 레코드에 속한 값이라 레코드가 없어지면 같이
/// 없어진다) — 프론트(SigningKeysSection)가 삭제 전에 이 사실을 확인받는다. Android 서명 비밀번호를
/// 등록해 뒀다면(android_signing) keychain 항목도 함께 지운다 — 참조를 잃은 비밀번호를 keychain 에
/// 영영 남겨두지 않는다(best-effort, 실패해도 레코드 삭제 자체는 이미 끝난 뒤라 무시해도 안전하다).
#[tauri::command]
pub async fn remove_signing_key(app: AppHandle, key_id: String) -> Result<(), String> {
    let base_dir = signing_base_dir(&app)?;
    let mut keys = signing::load_signing_keys(&base_dir)?;
    let before = keys.len();
    let removed_android_signing = keys
        .iter()
        .find(|k| k.id == key_id)
        .and_then(|k| k.android_signing.clone());
    keys.retain(|k| k.id != key_id);
    if keys.len() == before {
        return Err("등록된 서명키를 찾지 못했어요.".to_string());
    }
    signing::save_signing_keys(&base_dir, &keys)?;
    if let Some(cfg) = removed_android_signing {
        signing::forget_android_signing_secrets(&cfg);
    }
    Ok(())
}

/// Android release 자동 서명 비밀번호 등록(다음 단계) — kind ==
/// AndroidKeystore 인 서명키에만 허용한다. 비밀번호는 이 커맨드를 넘어가면(spawn_blocking 클로저 안)
/// 메모리에서만 잠깐 머물고, 실제 저장은 macOS keychain 이 담당한다(signing::register_android_signing).
/// 반환하는 SigningKeyRecord 에는 keychain 서비스 이름(참조)만 있을 뿐 비밀번호 원문은 없다.
#[tauri::command]
pub async fn register_android_signing(
    app: AppHandle,
    key_id: String,
    key_alias: String,
    store_password: String,
    key_password: String,
) -> Result<SigningKeyRecord, String> {
    let base_dir = signing_base_dir(&app)?;
    let mut keys = signing::load_signing_keys(&base_dir)?;
    let existing = keys
        .iter()
        .find(|k| k.id == key_id)
        .ok_or_else(|| "등록된 서명키를 찾지 못했어요.".to_string())?;
    if existing.kind != SigningKeyKind::AndroidKeystore {
        return Err("Android keystore 파일에만 서명 비밀번호를 등록할 수 있어요.".to_string());
    }
    let previous_android_signing = existing.android_signing.clone();
    // 등록 시점 인증서 겉정보(만료일/SHA-256) 추출에 쓸 실제 keystore 경로 — 볼트 사본이 있으면 그걸
    // 우선 쓴다(자체 완결 원칙, model.rs::SigningKeyRecord::vault_path 문서 참고). 이 기능 이전에
    // 등록된 레코드처럼 vault_path 가 없으면 원본(file_path)으로 물러난다. 파일이 그 사이 옮겨졌거나
    // 지워졌어도 signing::read_android_keystore_cert_metadata 가 조용히 (None, None) 으로 물러난다 —
    // 등록 자체는 계속 진행된다.
    let keystore_path = PathBuf::from(existing.vault_path.as_deref().unwrap_or(existing.file_path.as_str()));

    let key_id_for_keychain = key_id.clone();
    let join_result = tauri::async_runtime::spawn_blocking(move || {
        signing::register_android_signing(&keystore_path, &key_id_for_keychain, &key_alias, &store_password, &key_password)
    })
    .await
    .map_err(|e| format!("서명 비밀번호를 저장하는 중 문제가 발생했어요: {e}"))?;
    let config = join_result?;

    let key = keys
        .iter_mut()
        .find(|k| k.id == key_id)
        .ok_or_else(|| "등록된 서명키를 찾지 못했어요.".to_string())?;
    key.android_signing = Some(config.clone());
    let updated = key.clone();
    signing::save_signing_keys(&base_dir, &keys)?;

    // alias 를 바꿔 다시 등록한 경우 keychain account 가 달라져 이전 항목이 고아가 된다 — 새 항목 저장이
    // 이미 끝난 뒤에 best-effort 로 정리한다(성공 여부와 무관하게 새 비밀번호는 이미 안전하게 저장됨).
    if let Some(prev) = previous_android_signing {
        if prev.keychain_account != config.keychain_account {
            signing::forget_android_signing_secrets(&prev);
        }
    }

    Ok(updated)
}

/// 서명키를 프로젝트에 연결한다(다대다 — 하나의 인증서를 여러 앱에 쓸 수 있다). project_id 존재
/// 확인까지 한다(find_project) — 등록 안 된 프로젝트에 연결해 두면 나중에 찾을 방법이 없어진다.
#[tauri::command]
pub async fn link_signing_key(
    app: AppHandle,
    key_id: String,
    project_id: String,
) -> Result<SigningKeyRecord, String> {
    let base_dir = signing_base_dir(&app)?;
    find_project(&app, &project_id)?;
    let mut keys = signing::load_signing_keys(&base_dir)?;
    let key = keys
        .iter_mut()
        .find(|k| k.id == key_id)
        .ok_or_else(|| "등록된 서명키를 찾지 못했어요.".to_string())?;
    if !key.linked_project_ids.contains(&project_id) {
        key.linked_project_ids.push(project_id);
    }
    let updated = key.clone();
    signing::save_signing_keys(&base_dir, &keys)?;
    Ok(updated)
}

/// 서명키 연결 해제 — 이 프로젝트에서만 떼어낸다(레코드 자체는 남아 다른 프로젝트/미연결 목록에서
/// 계속 보인다). 프로젝트가 이미 삭제됐어도(project_id 가 더는 존재하지 않아도) 남은 연결을 정리할
/// 수 있어야 하므로 link_signing_key 와 달리 find_project 검증은 하지 않는다.
#[tauri::command]
pub async fn unlink_signing_key(
    app: AppHandle,
    key_id: String,
    project_id: String,
) -> Result<SigningKeyRecord, String> {
    let base_dir = signing_base_dir(&app)?;
    let mut keys = signing::load_signing_keys(&base_dir)?;
    let key = keys
        .iter_mut()
        .find(|k| k.id == key_id)
        .ok_or_else(|| "등록된 서명키를 찾지 못했어요.".to_string())?;
    key.linked_project_ids.retain(|id| id != &project_id);
    let updated = key.clone();
    signing::save_signing_keys(&base_dir, &keys)?;
    Ok(updated)
}

// ── 서명키/스토어 키 자동 탐색(다음 단계, keychain 이관 옵션 A) ──────────────────────────────
// 실제 스캔/파싱/keychain 이관 로직은 key_scan.rs 가 담당한다 — 여기는 base_dir 조회 + project_id 존재
// 확인만 한다(위 서명키 관리 커맨드들과 같은 "엔진 원칙" 경계). 무료 오픈소스라 게이트는 없다
// (signing_base_dir 문서 참고).

/// 개발 머신의 고정 경로들(key_scan.rs::scan_roots)을 스캔해 Android keystore/.p8 후보를 찾는다.
/// 파일시스템을 여러 곳 훑어 몇 초 걸릴 수 있어 spawn_blocking 으로 실행한다(run_preflight 와 동일
/// 이유). $HOME 을 못 구하면(사실상 없는 경우) 에러로 알린다.
#[tauri::command]
pub async fn scan_signing_keys() -> Result<Vec<FoundKey>, String> {
    let home = std::env::var("HOME").map_err(|_| "홈 폴더 위치를 확인하지 못했어요.".to_string())?;
    let home_path = PathBuf::from(home);
    tauri::async_runtime::spawn_blocking(move || key_scan::scan(&home_path))
        .await
        .map_err(|e| format!("서명키를 찾는 중 문제가 발생했어요: {e}"))
}

/// 스캔으로 찾은 keystore/.p8 을 "등록"하기 전에 클라우드 온디맨드(다운로드 전) 상태인지 미리
/// 확인한다(signing::inspect_key_source, stat 만 사용 — 파일을 열거나 다운로드를 유발하지 않는다).
/// import_found_android_signing 이 내부적으로 거치는 copy_keystore_into_vault 가 최대 ~31초 재시도
/// 하다 실패하는 것을 사용자가 매번 기다리지 않도록, 프론트(SigningKeysSection.tsx::FoundKeysPanel::
/// handleImportClick)가 실제 가져오기 전에 먼저 이 커맨드로 판정해 즉시 안내한다(리뷰 지적). 아직
/// 등록/연결 이전 시점의 순수 조회라 project_id 확인은 하지 않는다. stat 만 쓰는 단일 syscall 이라
/// spawn_blocking 을 쓰지 않는다(get_project_app_id 와 동일 관례).
#[tauri::command]
pub async fn inspect_key_source(path: String) -> Result<KeySourceInfo, String> {
    signing::inspect_key_source(Path::new(&path))
}

/// 클라우드 온디맨드 파일의 위치를 Finder 에서 강조 표시한다(reveal, `open -R <path>`) — 파일을 열거나
/// 다운로드를 유발하지 않는다(Finder 창이 그 파일을 선택한 채로 뜰 뿐). inspect_key_source 가 "아직
/// 다운로드되지 않았어요" 로 판정했을 때 프론트가 [Finder에서 열기] 버튼으로 이 커맨드를 부른다.
/// scan_signing_keys 가 이미 FoundKey.path 를 프론트에 내려주므로(표면 축소 예외, key_scan.rs 파일
/// 상단 주석) 그 값을 그대로 받는다. open_keystore_vault/open_external_url 과 동일하게 고정 argv
/// (Command::new("open"))만 쓰고 셸을 거치지 않는다.
#[tauri::command]
pub async fn reveal_signing_key_in_finder(path: String) -> Result<(), String> {
    let join_result = tauri::async_runtime::spawn_blocking(move || {
        let status = std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .status()
            .map_err(|e| format!("Finder에서 열지 못했어요: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("Finder에서 여는 데 실패했어요.".to_string())
        }
    })
    .await
    .map_err(|e| format!("Finder에서 여는 중 문제가 발생했어요: {e}"))?;
    join_result
}

/// 스캔으로 찾은 Android keystore 를 이 프로젝트에 등록 + 연결한다(옵션 A: key.properties 에 비밀번호가
/// 있으면 keychain 으로 자동 이관). 실제 로직은 key_scan::import_android_signing 이 담당 — 그 함수가
/// 프론트가 스캔 시점에 봤던 passwordsAvailable 을 그대로 믿지 않고 key.properties 를 다시 읽는다
/// (TOCTOU 방지). project_id 존재 확인은 link_signing_key 와 동일하게 여기서 먼저 한다.
#[tauri::command]
pub async fn import_found_android_signing(
    app: AppHandle,
    keystore_path: String,
    project_id: String,
) -> Result<ImportAndroidSigningResult, String> {
    let base_dir = signing_base_dir(&app)?;
    let vault_dir = store::keystore_vault_dir(&app)?;
    find_project(&app, &project_id)?;
    let join_result = tauri::async_runtime::spawn_blocking(move || {
        key_scan::import_android_signing(&base_dir, &vault_dir, Path::new(&keystore_path), &project_id)
    })
    .await
    .map_err(|e| format!("서명키를 가져오는 중 문제가 발생했어요: {e}"))?;
    join_result
}

/// 홑파일 keystore(등록 당시 옆에 key.properties 가 없는 경우)를 프로젝트에 등록·연결한 "다음" 자동으로
/// 시도하는 비밀번호 채움(확정된 설계 결정) — 그 프로젝트 자체의 key.properties
/// (key_scan::find_project_key_properties, "<repo_path>/android/key.properties")에서 비밀번호를 찾아
/// storeFile 이 이 keystore 로 정확히 resolve 될 때만(안전 매칭) keychain 에 자동 이관한다. 프론트
/// (SigningKeysSection::handleAddKey)가 register_signing_key + link_signing_key 로 등록·연결까지 마친
/// "다음", kind == android_keystore 이고 아직 androidSigning 이 없을 때만 호출한다 — 불일치/파일없음이면
/// imported:false 를 돌려주고 프론트는 기존처럼 수동 입력 폼으로 폴백한다(추측 금지).
#[tauri::command]
pub async fn autofill_android_signing(
    app: AppHandle,
    key_id: String,
    project_id: String,
) -> Result<ImportAndroidSigningResult, String> {
    let base_dir = signing_base_dir(&app)?;
    let project = find_project(&app, &project_id)?;
    let repo_path = PathBuf::from(&project.repo_path);
    let join_result = tauri::async_runtime::spawn_blocking(move || {
        key_scan::autofill_android_signing_from_project(&base_dir, &repo_path, &key_id)
    })
    .await
    .map_err(|e| format!("서명 비밀번호를 자동으로 채우는 중 문제가 발생했어요: {e}"))?;
    join_result
}

/// 스캔으로 찾은 .p8 을 "발견 기록"만 저장한다(가벼움, keychain 이관 없음) — .p8 은 아직 소비처가
/// 없어(로드맵 #6 스토어 자동 업로드가 나중에 사용) 경로·Key ID·종류만 남겨 둔다.
#[tauri::command]
pub async fn register_found_store_key(
    app: AppHandle,
    path: String,
    key_id: String,
    subtype: P8Subtype,
) -> Result<FoundStoreKeyRecord, String> {
    let base_dir = signing_base_dir(&app)?;
    let join_result = tauri::async_runtime::spawn_blocking(move || {
        key_scan::register_found_store_key(&base_dir, &path, &key_id, subtype)
    })
    .await
    .map_err(|e| format!("스토어 키를 기록하는 중 문제가 발생했어요: {e}"))?;
    join_result
}

/// 이미 "발견 기록"된 .p8 목록 — 프론트가 스캔 결과에서 이미 기록된 항목을 "기록됨"으로 표시하는 데
/// 쓴다(SigningKeysSection.tsx::FoundKeysPanel).
#[tauri::command]
pub async fn list_found_store_keys(app: AppHandle) -> Result<Vec<FoundStoreKeyRecord>, String> {
    let base_dir = signing_base_dir(&app)?;
    key_scan::load_found_store_keys(&base_dir)
}

/// 등록된 프로젝트의 Android applicationId(우선)/namespace(폴백) — 서명키 체크리스트 화면(앱 라벨,
/// SigningKeysSection.tsx)에 쓴다. 새 로직 없음: key_scan::find_app_id_in_project_dir 를 그대로
/// 재사용한다 — project.repo_path(pubspec.yaml 이 있는 실제 Flutter 프로젝트 루트)의 "android" 하위가
/// pubspec.rs::detect_platforms 가 Android 플랫폼을 감지하는 경로와 동일해, 그 디렉터리를 그대로 넘기면
/// key.properties 없이도(이미 프로젝트 루트를 알고 있으니 climbing 불필요) applicationId 를 곧장 찾을 수
/// 있다. android 폴더가 없거나 build.gradle 파싱에 실패해도 조용히 None — 화면은 앱 이름만 보여준다
/// (하드 에러 아님, key_scan.rs 전체의 관대한 처리 철학과 동일).
#[tauri::command]
pub async fn get_project_app_id(app: AppHandle, project_id: String) -> Result<Option<String>, String> {
    let project = find_project(&app, &project_id)?;
    let android_dir = PathBuf::from(&project.repo_path).join("android");
    Ok(key_scan::find_app_id_in_project_dir(&android_dir))
}

// ── 앱 설정(1차, 설정 화면) ───────────────────────────────────────────────────
// 실제 IO/Flutter 자동 감지·검증은 settings.rs 가 담당한다(위 서명키 관리 커맨드들과 같은 "엔진 원칙"
// 경계 — 여기는 AppHandle 해석 + spawn_blocking(외부 프로세스를 쓰는 것만) 만 한다).

/// 설정 화면 초기 로드 — 파일이 없거나 손상됐어도 기본값을 돌려준다(하드 에러 없음, settings.rs::
/// load_settings_from_dir 문서 참고).
#[tauri::command]
pub async fn get_settings(app: AppHandle) -> Result<AppSettings, String> {
    settings::load_settings(&app)
}

/// 설정 화면이 필드 하나가 바뀔 때마다(자동 저장, 별도 "저장" 버튼 없음) 전체 스냅샷을 저장한다 —
/// 필드가 몇 개 안 돼 read-modify-write 를 프론트(useSettings)에서 하는 편이 부분 patch 커맨드를 따로
/// 만드는 것보다 단순하다.
#[tauri::command]
pub async fn set_settings(app: AppHandle, new_settings: AppSettings) -> Result<AppSettings, String> {
    settings::save_settings(&app, &new_settings)?;
    Ok(new_settings)
}

/// Flutter SDK 자동 감지("자동 감지" 버튼) — PATH(보강 포함) 또는 흔한 설치 위치에서 찾는다. 파일시스템
/// 탐색 + `which` 실행이 있어 run_preflight 와 동일하게 spawn_blocking 으로 async 런타임을 막지 않는다.
/// 못 찾으면 Ok(None)(에러 아님 — 프론트가 "직접 입력해 주세요" 안내로 바꾼다).
#[tauri::command]
pub async fn detect_flutter_sdk() -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(settings::detect_flutter_path)
        .await
        .map_err(|e| format!("Flutter를 찾는 중 문제가 발생했어요: {e}"))
}

/// 주어진 경로가 실제 Flutter 인지 확인한다 — 설정 화면이 입력/감지된 경로의 유효성을 "유효하면
/// flutter --version 첫 줄 표시"에 쓴다.
#[tauri::command]
pub async fn check_flutter_path(path: String) -> Result<String, String> {
    let join_result = tauri::async_runtime::spawn_blocking(move || settings::check_flutter_version(&path))
        .await
        .map_err(|e| format!("Flutter 경로를 확인하는 중 문제가 발생했어요: {e}"))?;
    join_result
}

/// 서명키 안전 보관 볼트 폴더 경로(표시용) — store::keystore_vault_dir 그대로(없으면 여기서 만들어짐).
#[tauri::command]
pub async fn get_keystore_vault_path(app: AppHandle) -> Result<String, String> {
    let vault_dir = store::keystore_vault_dir(&app)?;
    Ok(vault_dir.to_string_lossy().to_string())
}

/// 서명키 안전 보관 볼트 폴더를 Finder 로 연다(설정 화면) — 경로는 프론트에서 받지 않고 항상
/// store::keystore_vault_dir(&app) 가 돌려주는 값만 연다(엔진 원칙 — 임의 경로를 열지 않는다). macOS
/// `open` 커맨드를 고정 인자로 실행할 뿐 셸을 거치지 않는다(child_env.rs::kill_process_group 과 동일한
/// "Command::new + 고정 argv" 원칙).
#[tauri::command]
pub async fn open_keystore_vault(app: AppHandle) -> Result<(), String> {
    let vault_dir = store::keystore_vault_dir(&app)?;
    let join_result = tauri::async_runtime::spawn_blocking(move || {
        let status = std::process::Command::new("open")
            .arg(&vault_dir)
            .status()
            .map_err(|e| format!("Finder에서 열지 못했어요: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("Finder에서 여는 데 실패했어요.".to_string())
        }
    })
    .await
    .map_err(|e| format!("Finder에서 여는 중 문제가 발생했어요: {e}"))?;
    join_result
}

/// 설정 화면 "정보" 섹션의 외부 링크(GitHub 저장소)를 시스템 기본 브라우저로 연다. https:// 로
/// 시작하는 값만 허용한다(로컬 파일·커스텀 스킴 실행 방지) — 지금은 GitHub 저장소 링크 하나뿐이라
/// 화이트리스트까지는 과하지만 최소한 스킴은 제한한다. open_keystore_vault 와 동일하게 셸을 거치지
/// 않는다.
#[tauri::command]
pub async fn open_external_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("올바르지 않은 링크예요.".to_string());
    }
    let join_result = tauri::async_runtime::spawn_blocking(move || {
        let status = std::process::Command::new("open")
            .arg(&url)
            .status()
            .map_err(|e| format!("브라우저를 열지 못했어요: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("브라우저를 여는 데 실패했어요.".to_string())
        }
    })
    .await
    .map_err(|e| format!("브라우저를 여는 중 문제가 발생했어요: {e}"))?;
    join_result
}

/// CLI 서브커맨드 문서 목록(3단계, bildorak-cli) — 설정 화면 "CLI / 자동화" 섹션에 쓴다. build.rs::
/// cli_manifest() 를 그대로 반환할 뿐이다 — 그 함수 문서 참고: clap --help 문구(cli.rs)와 이 화면
/// 둘 다 같은 데이터를 쓰는 단일 소스다. get_app_version 과 마찬가지로 컴파일 타임 상수라 외부 IO 없음.
#[tauri::command]
pub async fn get_cli_manifest() -> Vec<CliCommandDoc> {
    build::cli_manifest()
}

/// 앱 버전(Cargo.toml 의 version — tauri.conf.json/package.json 과 항상 같은 값으로 맞춰 관리한다) —
/// 설정 화면 "정보" 섹션에 쓴다. 컴파일 타임 상수라 외부 IO 가 없다.
#[tauri::command]
pub async fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
