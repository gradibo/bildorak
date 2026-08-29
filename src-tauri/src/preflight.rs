// preflight.rs — read-only 빌드 준비 점검. 등록된 프로젝트 폴더 하위만 보고, 외부 명령은 고정
// argv 로만 실행한다(엔진 원칙 — Command::new + 인자 배열, 셸 경유 금지). 프론트가
// 넘기는 값은 project_id 뿐이고, 실제 실행 경로/명령은 전부 여기 서버(Rust)측 고정값이다.
//
// 자식 프로세스 env 는 child_env.rs 의 공유 allowlist/fallback 정책을 그대로 쓴다(build.rs 의 실제
// 빌드 실행과 같은 정책 — 점검은 통과인데 실제 빌드는 다른 이유로 실패하는 drift 를 막는다).
// .env/서명 키 파일은 여기서 읽지도 접근하지도 않는다.

use crate::child_env;
use crate::model::{CheckItem, CheckStatus, OsScope, Platform, PreflightRun, ProjectRecord};
use chrono::Utc;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// env 는 child_env 공유 정책, stdio 는 여기서만 버림(점검은 exit code 만 보면 된다).
fn allowlisted_command(bin: &str) -> Command {
    let mut cmd = Command::new(bin);
    child_env::apply_allowlisted_env(&mut cmd);
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    cmd
}

/// 고정 argv 프로세스를 새 process group 으로 띄우고 최대 timeout 만큼만 기다린다 — 넘으면 그룹
/// 전체를 죽이고 실패로 본다(단순 타임아웃 처리에서 한 발
/// 더 나가 process group 전체를 정리한다 — flutter/xcodebuild 같은 래퍼가 만드는 손자 프로세스가
/// 타임아웃 이후에도 고아로 남는 것을 막기 위함). 셸을 거치지 않으므로
/// injection 여지가 없다.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> bool {
    child_env::spawn_in_new_process_group(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return false,
    };
    let pid = child.id();
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if start.elapsed() > timeout {
                    child_env::kill_process_group(pid);
                    let _ = child.wait();
                    return false;
                }
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(_) => return false,
        }
    }
}

/// `bin` `args` 만 실행하는 버전 점검 — 사용자 입력 없음(고정 문자열). 20초 넘으면 fail.
fn check_tool(label: &str, bin: &str, args: &[&str], os: OsScope) -> CheckItem {
    let mut cmd = allowlisted_command(bin);
    cmd.args(args);
    if run_with_timeout(cmd, Duration::from_secs(20)) {
        CheckItem {
            label: label.to_string(),
            status: CheckStatus::Pass,
            message: format!("{bin} 명령이 정상적으로 설치돼 있어요."),
            next_action: None,
            os,
        }
    } else {
        CheckItem {
            label: label.to_string(),
            status: CheckStatus::Fail,
            message: format!("{bin} 명령을 찾지 못했거나 응답하지 않았어요."),
            next_action: Some(format!("Mac에 {bin} 이 설치·설정돼 있는지 확인하세요.")),
            os,
        }
    }
}

/// ios/ android/ 폴더 존재 여부 — 등록 시점 이후 폴더가 지워졌을 수 있어 매번 다시 본다.
fn check_platform_folder(repo_path: &Path, platform: Platform) -> CheckItem {
    let (folder, label) = match platform {
        Platform::Ios => ("ios", "iOS 폴더"),
        Platform::Android => ("android", "Android 폴더"),
    };
    let os = match platform {
        Platform::Ios => OsScope::Macos,
        Platform::Android => OsScope::All,
    };
    let exists = repo_path.join(folder).is_dir();
    CheckItem {
        label: label.to_string(),
        status: if exists { CheckStatus::Pass } else { CheckStatus::Fail },
        message: if exists {
            format!("{folder}/ 폴더가 있어요.")
        } else {
            format!("{folder}/ 폴더를 찾지 못했어요.")
        },
        next_action: if exists {
            None
        } else {
            Some(format!("flutter create 로 {folder} 플랫폼을 추가해야 해요."))
        },
        os,
    }
}

/// CocoaPods — iOS 플랫폼일 때만 의미가 있다(macOS 전용).
fn check_cocoapods() -> CheckItem {
    check_tool("CocoaPods", "pod", &["--version"], OsScope::Macos)
}

/// Android SDK — child_env::resolve_android_home() 이 환경변수(ANDROID_HOME/ANDROID_SDK_ROOT) 또는
/// macOS 기본 설치 경로(~/Library/Android/sdk)까지 확인한다. JAVA_HOME 도 child_env::resolve_java_home()
/// 으로 같은 fallback(환경변수 → /usr/libexec/java_home → Android Studio 내장 JBR)을 쓴다 — GUI 앱은
/// 셸 rc 를 상속하지 않아 터미널에서만 되던 게 false warning 으로 뜨는 문제(설계 요구사항)를
/// build.rs 의 실제 빌드 실행 env 와 동일한 기준으로 해소한다.
fn check_android_sdk() -> CheckItem {
    let sdk_found = child_env::resolve_android_home().is_some();
    let java_home_set = child_env::resolve_java_home().is_some();

    if !sdk_found {
        return CheckItem {
            label: "Android SDK".to_string(),
            status: CheckStatus::Warn,
            message: "Android SDK 경로를 찾지 못했어요.".to_string(),
            next_action: Some(
                "ANDROID_HOME 환경변수를 설정하거나 Android Studio에서 SDK를 설치하세요."
                    .to_string(),
            ),
            os: OsScope::All,
        };
    }
    if !java_home_set {
        return CheckItem {
            label: "Android SDK".to_string(),
            status: CheckStatus::Warn,
            message: "Android SDK는 찾았지만 JAVA_HOME이 설정돼 있지 않아요.".to_string(),
            next_action: Some("JAVA_HOME 환경변수에 설치된 JDK 경로를 설정하세요.".to_string()),
            os: OsScope::All,
        };
    }
    CheckItem {
        label: "Android SDK".to_string(),
        status: CheckStatus::Pass,
        message: "Android SDK와 JAVA_HOME이 정상적으로 설정돼 있어요.".to_string(),
        next_action: None,
        os: OsScope::All,
    }
}

/// 디스크 여유 공간 — repoPath 가 속한 볼륨 기준. 임계값은 다른 상시 모니터링 프로세스의 disk_low
/// 기준(여유 < 5GB)과 통일, 15GB 미만은 주의로만 표시.
fn check_disk_space(repo_path: &Path) -> CheckItem {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut best: Option<&sysinfo::Disk> = None;
    let mut best_len = 0usize;
    for disk in disks.list() {
        let mount = disk.mount_point();
        if repo_path.starts_with(mount) {
            let len = mount.as_os_str().len();
            if len >= best_len {
                best_len = len;
                best = Some(disk);
            }
        }
    }
    match best {
        Some(disk) => {
            let available_gb = disk.available_space() as f64 / 1_073_741_824.0;
            if available_gb < 5.0 {
                CheckItem {
                    label: "디스크 여유 공간".to_string(),
                    status: CheckStatus::Fail,
                    message: format!("여유 공간이 {available_gb:.1}GB 밖에 없어요."),
                    next_action: Some("빌드 전에 불필요한 파일을 정리해서 공간을 확보하세요.".to_string()),
                    os: OsScope::All,
                }
            } else if available_gb < 15.0 {
                CheckItem {
                    label: "디스크 여유 공간".to_string(),
                    status: CheckStatus::Warn,
                    message: format!("여유 공간이 {available_gb:.1}GB 예요. 빌드 중 부족할 수 있어요."),
                    next_action: Some("가능하면 여유 공간을 더 확보하세요.".to_string()),
                    os: OsScope::All,
                }
            } else {
                CheckItem {
                    label: "디스크 여유 공간".to_string(),
                    status: CheckStatus::Pass,
                    message: format!("여유 공간이 {available_gb:.1}GB 예요."),
                    next_action: None,
                    os: OsScope::All,
                }
            }
        }
        None => CheckItem {
            label: "디스크 여유 공간".to_string(),
            status: CheckStatus::Warn,
            message: "디스크 여유 공간을 확인하지 못했어요.".to_string(),
            next_action: None,
            os: OsScope::All,
        },
    }
}

/// settings.rs 에 등록된 Flutter 경로가 있으면 그 경로로 "--version"을 실행해 확인하고, 없으면(미설정)
/// resolve_flutter_bin 이 그대로 물려주는 "flutter"(PATH 탐색)로 확인한다 — build.rs::start_build 가
/// 실제 빌드에 쓰는 경로 결정 로직(settings::resolve_flutter_bin)과 항상 같은 기준을 써서, "점검은
/// 통과했는데 실제 빌드는 다른 flutter 를 쓰는" drift 를 막는다(child_env.rs 파일 상단 원칙과 동일).
fn check_flutter_tool(base_dir: &Path) -> CheckItem {
    let bin = crate::settings::resolve_flutter_bin(base_dir);
    check_tool("Flutter 도구", &bin, &["--version"], OsScope::All)
}

/// 프로젝트 없이 전역 빌드 환경만 점검한다(CLI `bildorak-cli doctor` 전용, additive) — 위
/// check_tool/check_cocoapods/check_android_sdk 를 그대로 재사용한다. ios/android 폴더 존재나 디스크
/// 여유공간처럼 프로젝트 경로가 있어야 의미가 있는 항목(run() 참고)은 "프로젝트 없음"이라는 이 함수의
/// 성격과 맞지 않아 포함하지 않는다 — 등록된 앱 각각의 상태는 run()/CLI `status` 가 담당한다.
pub fn check_environment(base_dir: &Path) -> Vec<CheckItem> {
    vec![
        check_flutter_tool(base_dir),
        check_tool("Xcode 도구", "xcodebuild", &["-version"], OsScope::Macos),
        check_cocoapods(),
        check_android_sdk(),
    ]
}

/// 등록된 프로젝트 하나에 대한 전체 점검 실행 — project.repoPath 는 등록 시점에 실측 검증된
/// 값만 저장돼 있으므로 여기서 다시 신뢰하되, 폴더/도구 존재는 실행마다 새로 확인한다.
pub fn run(project: &ProjectRecord) -> PreflightRun {
    let started_at = Utc::now().to_rfc3339();
    let repo_path = Path::new(&project.repo_path);

    let mut checks: Vec<CheckItem> = Vec::new();
    checks.push(check_tool("Flutter 도구", "flutter", &["--version"], OsScope::All));

    for platform in &project.platforms {
        checks.push(check_platform_folder(repo_path, *platform));
    }
    if project.platforms.contains(&Platform::Ios) {
        checks.push(check_tool("Xcode 도구", "xcodebuild", &["-version"], OsScope::Macos));
        checks.push(check_cocoapods());
    }
    if project.platforms.contains(&Platform::Android) {
        checks.push(check_android_sdk());
    }
    checks.push(check_disk_space(repo_path));

    let overall_status = crate::model::overall_status_of(&checks);
    PreflightRun {
        id: Uuid::new_v4().to_string(),
        project_id: project.id.clone(),
        started_at,
        finished_at: Utc::now().to_rfc3339(),
        overall_status,
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 이 머신의 실제 Flutter/Xcode/CocoaPods/Android SDK 설치 여부(pass/warn/fail)는 실행 환경마다
    /// 달라 단정할 수 없다 — 대신 항목 구성(라벨 4개, 순서 고정)이 항상 같은지만 고정한다. CLI
    /// `doctor` 가 이 순서 그대로 출력한다.
    #[test]
    fn check_environment_returns_fixed_label_set_regardless_of_machine_state() {
        let base_dir = std::env::temp_dir().join(format!("bildorak-doctor-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&base_dir).expect("failed to create temp base dir");

        let checks = check_environment(&base_dir);
        let labels: Vec<&str> = checks.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["Flutter 도구", "Xcode 도구", "CocoaPods", "Android SDK"]);

        let _ = std::fs::remove_dir_all(&base_dir);
    }
}
