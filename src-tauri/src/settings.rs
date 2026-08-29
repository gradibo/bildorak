// settings.rs — settings.json 읽기/쓰기(app config dir, store.rs::app_config_dir 와 같은 폴더) + Flutter
// SDK 경로 자동 감지/검증. 설정 화면(1차)이 쓰는 유일한 IO 진입점이다 — model.rs::AppSettings 는 데이터
// 모양만(파일 상단 원칙, model.rs 문서 참고).
//
// build.rs 는 AppHandle 을 모르는 경계를 유지한다(build.rs 파일 상단 주석) — 그래서 이 파일의 읽기
// 함수는 base_dir: &Path 를 받는 형태(load_settings_from_dir)를 기본으로 두고, commands.rs 가 부르는
// AppHandle 버전(load_settings)은 그 위에 app_config_dir 해석만 얹은 얇은 wrapper 다. resolve_flutter_bin
// 도 같은 이유로 base_dir 만 받는다 — build.rs::start_build 가 이 함수 하나로 실제 빌드에 쓸 flutter
// 바이너리를 결정한다.

use crate::model::AppSettings;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::AppHandle;

const SETTINGS_FILE: &str = "settings.json";

fn settings_file_path(base_dir: &Path) -> PathBuf {
    base_dir.join(SETTINGS_FILE)
}

/// base_dir 하나만으로 설정을 읽는다 — 파일 없음/빈 파일/파싱 실패 전부 기본값으로 조용히 물러난다
/// (설정 파일 하나 손상됐다고 빌드·앱 실행 전체가 막히면 안 된다 — build.rs 의 self-heal 정신과
/// 같지만, 설정은 재입력 비용이 낮아 손상 파일 백업까지는 하지 않는다).
pub fn load_settings_from_dir(base_dir: &Path) -> AppSettings {
    let path = settings_file_path(base_dir);
    let Ok(raw) = fs::read_to_string(&path) else {
        return AppSettings::default();
    };
    if raw.trim().is_empty() {
        return AppSettings::default();
    }
    serde_json::from_str(&raw).unwrap_or_default()
}

/// commands.rs::get_settings 가 쓰는 AppHandle 버전 — app_config_dir 해석 후 load_settings_from_dir 로.
pub fn load_settings(app: &AppHandle) -> Result<AppSettings, String> {
    Ok(load_settings_from_dir(&crate::store::app_config_dir(app)?))
}

/// 설정을 저장한다(store.rs::write_json_atomic 재사용 — projects.json/build_jobs.json 과 동일한 원자적
/// 쓰기, 저장 도중 죽어도 반쪽 파일이 안 남는다).
pub fn save_settings(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let base_dir = crate::store::app_config_dir(app)?;
    let path = settings_file_path(&base_dir);
    let raw = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("저장할 설정 데이터를 만들지 못했어요: {e}"))?;
    crate::store::write_json_atomic(&path, &raw).map_err(|e| format!("설정을 저장하지 못했어요: {e}"))
}

/// build.rs 가 실제 빌드 실행 시 쓸 flutter 바이너리 경로/이름 — 설정된 경로가 있고 실제 파일이면 그
/// 경로를 그대로 쓰고, 없거나(미설정) 파일이 아니면(경로가 옮겨짐 등) 기존 그대로 "flutter"(PATH
/// 탐색)로 물러난다. 이 폴백 덕분에 이 기능 이전 동작은 100% 그대로 유지된다(무회귀, 아래 tests 모듈
/// 참고) — build.rs::resolve_command 의 고정 argv 는 이 함수와 무관하게 그대로다(bin 문자열 하나만
/// 바뀔 뿐).
pub fn resolve_flutter_bin(base_dir: &Path) -> String {
    match load_settings_from_dir(base_dir).flutter_path {
        Some(p) if Path::new(&p).is_file() => p,
        _ => "flutter".to_string(),
    }
}

// ── Flutter SDK 경로 자동 감지/검증 ──────────────────────────────────────────────

/// `which flutter`(PATH, child_env::fixed_path_env() 로 보강 — build.rs 실제 빌드 실행이 쓰는 PATH 와
/// 동일 기준이라야 "설정에서는 찾았는데 빌드는 못 찾는" drift 가 없다) → `~/.flutter*/flutter/bin/flutter`
/// 글롭(fvm 등 버전 매니저가 흔히 쓰는 폴더명 패턴이라 정확한 이름 대신 접두사로 찾는다) 순으로 첫
/// 번째로 실존하는 절대경로를 돌려준다. 아무 것도 못 찾으면 None(에러 아님 — commands.rs 가 "직접
/// 입력해 주세요" 안내로 바꾼다).
pub fn detect_flutter_path() -> Option<String> {
    let mut which_cmd = Command::new("which");
    which_cmd.arg("flutter").env("PATH", crate::child_env::fixed_path_env()).stderr(Stdio::null());
    if let Ok(output) = which_cmd.output() {
        if output.status.success() {
            let found = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !found.is_empty() && Path::new(&found).is_file() {
                return Some(found);
            }
        }
    }

    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return None;
    }
    let Ok(entries) = fs::read_dir(&home) else {
        return None;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(".flutter") {
            continue;
        }
        let candidate = entry.path().join("flutter").join("bin").join("flutter");
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

/// 주어진 경로가 실제 flutter 실행 파일인지 확인하고 `flutter --version` 첫 줄을 돌려준다 — 설정
/// 화면이 "유효하면 표시"에 쓴다. 파일이 없거나 실행이 실패하면 비개발자 톤 에러(Err) — 사용자가
/// 직접 고르거나 입력한 경로라 왜 실패했는지 문구로 알려줘야 한다(preflight.rs::check_tool 은 "설치
/// 여부"만 보면 되는 고정 명령이라 여기와 에러 문구 성격이 다르다).
pub fn check_flutter_version(path: &str) -> Result<String, String> {
    if !Path::new(path).is_file() {
        return Err("이 경로에 파일이 없어요.".to_string());
    }
    let output = Command::new(path)
        .arg("--version")
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("flutter를 실행하지 못했어요: {e}"))?;
    if !output.status.success() {
        return Err("flutter --version 실행이 실패했어요. 올바른 Flutter 경로인지 확인해 주세요.".to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let first_line = text.lines().next().unwrap_or("").trim().to_string();
    if first_line.is_empty() {
        return Err("flutter --version 출력을 확인하지 못했어요.".to_string());
    }
    Ok(first_line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_base_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bildorak-settings-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("failed to create temp base dir");
        dir
    }

    #[test]
    fn resolve_flutter_bin_falls_back_to_path_when_no_settings_file() {
        let dir = temp_base_dir();
        assert_eq!(resolve_flutter_bin(&dir), "flutter");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_flutter_bin_falls_back_when_configured_path_missing() {
        let dir = temp_base_dir();
        let settings = AppSettings {
            flutter_path: Some("/nonexistent/flutter-binary-for-test".to_string()),
            ..AppSettings::default()
        };
        fs::write(dir.join(SETTINGS_FILE), serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(resolve_flutter_bin(&dir), "flutter");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_flutter_bin_uses_configured_path_when_file_exists() {
        let dir = temp_base_dir();
        let fake_flutter = dir.join("fake-flutter");
        fs::write(&fake_flutter, "#!/bin/sh\necho fake").unwrap();
        let settings = AppSettings {
            flutter_path: Some(fake_flutter.to_string_lossy().to_string()),
            ..AppSettings::default()
        };
        fs::write(dir.join(SETTINGS_FILE), serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(resolve_flutter_bin(&dir), fake_flutter.to_string_lossy().to_string());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_settings_from_dir_defaults_when_file_missing() {
        let dir = temp_base_dir();
        let settings = load_settings_from_dir(&dir);
        assert_eq!(settings.flutter_path, None);
        assert!(settings.build_notifications_enabled);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_settings_from_dir_self_heals_on_corrupt_json() {
        let dir = temp_base_dir();
        fs::write(dir.join(SETTINGS_FILE), "{ 이건 유효한 JSON 이 아니에요").unwrap();
        let settings = load_settings_from_dir(&dir);
        assert_eq!(settings.flutter_path, None);
        let _ = fs::remove_dir_all(&dir);
    }
}
