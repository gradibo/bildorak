// lib.rs - Tauri 앱 진입점. 1차(read-only: 프로젝트 등록 + preflight 점검) + 2차(로컬 빌드 실행)
// 커맨드를 노출한다. opener 플러그인은 사용처가 없어 제거했다(설계 원칙 - 표면 축소).
// 2단계(무료 오픈소스, 게이트 없음): 빌드 히스토리 커맨드(get_build_history) 추가 + 빌드
// 완료 macOS 알림을 위해 tauri-plugin-notification 등록(commands.rs::spawn_build_finish_notifier 가 씀).
// 다음 단계(무료 오픈소스, 게이트 없음): Android release 서명 자동 주입 - register_android_signing 커맨드로
// keychain 에 비밀번호를 등록해 두면 AndroidRelease 빌드가 자동으로 -P 서명 주입 + 빌드 후 서명 검증까지
// 한다(build.rs::start_build/spawn_build_job, signing.rs 의 keychain/jarsigner/keytool 함수들).
// 그 위: 서명키/스토어 키 자동 탐색 + keychain 이관(옵션 A) - scan_signing_keys 로 개발 머신 여기저기
// 흩어진 keystore/.p8 을 찾아 보여주고, import_found_android_signing 이 key.properties 의 비밀번호를
// 찾아 자동으로 keychain 에 이관한다(key_scan.rs). .p8 은 아직 소비처가 없어 register_found_store_key 로
// "발견 기록"만 남긴다(로드맵 #6 스토어 자동 업로드가 나중에 사용).
// 그 위: 홑파일 keystore 비밀번호 자동 채움 + keystore 안전 보관(볼트 복사) - register_signing_key 가
// Android keystore 등록 시 원본을 app_data_dir/keystores/ 로 복사해 두고(store.rs::keystore_vault_dir,
// signing.rs::copy_keystore_into_vault), autofill_android_signing 커맨드가 그 프로젝트 자체의
// key.properties(key_scan.rs::autofill_android_signing_from_project)에서 비밀번호를 찾아 keychain 에
// 자동 이관한다.
// 그 위: 설정 화면(1차) - Flutter SDK 경로/언어/테마/빌드 완료 알림/서명키 보관함 위치/정보(About).
// settings.rs 가 settings.json 읽기/쓰기 + Flutter 자동 감지/검증을 담당한다. build.rs::start_build 는
// settings.rs::resolve_flutter_bin 으로 설정된 flutter 경로가 있으면 그걸 쓰고, 없으면 기존 그대로
// PATH 의 "flutter"로 물러난다(무회귀). 빌드 완료 알림은 이 설정(buildNotificationsEnabled)으로
// commands.rs::start_build 가 게이트한다.
// 그 위: 자동 업데이트(Tauri 공식 updater) - 앱 시작 시 프론트(UpdateModal.tsx)가 조용히 GitHub
// Releases 의 latest.json 을 확인하고, 새 버전이 있으면 모달로 안내한다. updater/process 플러그인을
// 여기 등록하고, 실제 서명 검증(pubkey)/엔드포인트는 tauri.conf.json 의 plugins.updater 설정,
// 권한은 capabilities/default.json 의 updater:default/process:default 가 담당한다(이 파일은 등록만).
// 자동 확인 여부는 AppSettings::auto_update_check_enabled(기본 켬)로 프론트에서 게이트한다.

// 아래 mod 전부 pub - 3단계(bildorak-cli) 착수: 같은 크레이트의 2번째 bin
// (src/bin/cli.rs)이 AppHandle 없이 이 코어 모듈들을 직접 호출한다(paths::base_dir() + *_from_dir 계열
// 함수). 가시성만 넓히는 순수 additive 변경 - 각 모듈 내부 함수/구조체의 개별 pub 여부와 동작은 전혀
// 바뀌지 않는다(GUI 쪽 호출부·commands.rs 어댑터 계층 전부 무회귀).
pub mod build;
pub mod child_env;
pub mod commands;
pub mod key_scan;
pub mod model;
pub mod paths;
pub mod preflight;
pub mod pubspec;
pub mod releases;
pub mod settings;
pub mod signing;
pub mod store;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            commands::pick_project_folder,
            commands::register_project,
            commands::list_projects,
            commands::remove_project,
            commands::run_preflight,
            commands::start_build,
            commands::get_build_status,
            commands::cancel_build,
            commands::get_build_history,
            commands::pick_signing_key_file,
            commands::list_signing_keys,
            commands::register_signing_key,
            commands::remove_signing_key,
            commands::link_signing_key,
            commands::unlink_signing_key,
            commands::register_android_signing,
            commands::autofill_android_signing,
            commands::scan_signing_keys,
            commands::inspect_key_source,
            commands::reveal_signing_key_in_finder,
            commands::import_found_android_signing,
            commands::register_found_store_key,
            commands::list_found_store_keys,
            commands::get_project_app_id,
            commands::get_settings,
            commands::set_settings,
            commands::detect_flutter_sdk,
            commands::check_flutter_path,
            commands::get_keystore_vault_path,
            commands::open_keystore_vault,
            commands::open_external_url,
            commands::get_app_version,
            commands::get_cli_manifest,
            commands::list_releases,
            commands::create_release,
            commands::update_release,
            commands::delete_release,
            commands::get_project_current_version,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    // 앱 종료 시 이번 세션에서 우리가 띄운, 아직 안 끝난 빌드를 process group 째로 정리한다 - 창을
    // 닫아도 flutter/xcodebuild/gradle 자식이 고아로 남지 않게(설계 원칙 - 좀비 프로세스 방지).
    app.run(|_app_handle, event| {
        if matches!(
            event,
            tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }
        ) {
            build::kill_all_running_builds();
        }
    });
}
