// child_env.rs — 자식 프로세스(flutter/xcodebuild/pod/gradle 등)에 물려줄 환경과 process group 정책을
// 한 곳에 모은다. preflight.rs(read-only 점검)와 build.rs(실제 빌드 실행) 양쪽이 완전히 같은 안전
// 정책(env allowlist, JAVA_HOME/ANDROID_HOME fallback, process group)을 써야 하므로 여기 모아 두
// 구현이 서로 어긋나는 일(drift)을 막는다 — 관련 로직을 한 파일에 모아 두는 것이 안전하다는 판단이다.
//
// GUI 앱은 터미널과 달리 셸 rc(.zshrc/.bash_profile)를 상속하지 않는다 — JAVA_HOME/ANDROID_HOME 이
// 터미널에서만 잡히고 앱에선 못 잡히는 것이 흔한 원인이다(설계 요구사항). 그래서 env
// 변수만 믿지 않고 macOS 표준 탐지 도구/기본 설치 경로까지 fallback 으로 확인한다.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// 자식 프로세스에 그대로 넘길 환경변수 allowlist — secrets 상속 금지(확정된 설계 결정).
/// JAVA_HOME/ANDROID_HOME/ANDROID_SDK_ROOT 는 fallback 이 있어 아래 목록에서 빼고 별도로 처리한다.
const ENV_ALLOWLIST: &[&str] = &["HOME", "USER", "LOGNAME", "TMPDIR", "LANG"];

/// flutter/cocoapods 가 launchd 기본 PATH 에 없을 수 있어 흔한 설치 위치를 보강한다.
pub fn fixed_path_env() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut parts = vec![std::env::var("PATH").unwrap_or_default()];
    parts.push(format!("{home}/.flutter-stable/flutter/bin"));
    parts.push(format!("{home}/flutter/bin"));
    parts.push("/opt/homebrew/bin".to_string());
    parts.push("/usr/local/bin".to_string());
    parts.push("/usr/bin".to_string());
    parts.push("/bin".to_string());
    parts.join(":")
}

/// JAVA_HOME 실측 fallback. 우선순위: 이미 잡힌 환경변수(실존 검증) → `/usr/libexec/java_home`(macOS
/// 표준 JDK 탐지 도구, 실측 결과 `/opt/homebrew/Cellar/openjdk@17/...` 반환 확인) → Android Studio 내장
/// JBR(실측 경로 `/Applications/Android Studio.app/Contents/jbr/Contents/Home` 존재 확인,
/// 설계 요구사항).
pub fn resolve_java_home() -> Option<String> {
    if let Ok(value) = std::env::var("JAVA_HOME") {
        if Path::new(&value).is_dir() {
            return Some(value);
        }
    }
    if let Ok(output) = Command::new("/usr/libexec/java_home").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() && Path::new(&path).is_dir() {
                return Some(path);
            }
        }
    }
    let jbr_candidate = "/Applications/Android Studio.app/Contents/jbr/Contents/Home";
    if Path::new(jbr_candidate).is_dir() {
        return Some(jbr_candidate.to_string());
    }
    None
}

/// ANDROID_HOME fallback — 환경변수가 없어도 Android Studio 기본 설치 경로(~/Library/Android/sdk,
/// 실측 확인)가 있으면 그 경로를 쓴다. preflight 의 Android SDK 점검과 여기 build 실행 env 가 서로
/// 다른 기준을 쓰면 "점검은 통과인데 실제 빌드는 SDK 를 못 찾는" drift 가 생기므로 하나로 통일한다.
pub fn resolve_android_home() -> Option<String> {
    for key in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(value) = std::env::var(key) {
            if Path::new(&value).is_dir() {
                return Some(value);
            }
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let default_path = format!("{home}/Library/Android/sdk");
    if Path::new(&default_path).is_dir() {
        return Some(default_path);
    }
    None
}

/// jarsigner/keytool 실행 파일 경로 — resolve_java_home() 이 찾아준 JDK 하위 bin/<tool_name> 을 우선
/// 쓴다(Android 빌드에 쓰는 JAVA_HOME 과 서명 검증에 쓰는 JDK 가 서로 달라 "빌드는 JDK A 로, 검증은
/// JDK B 로" 하는 drift 가 생기는 걸 막는다). JAVA_HOME 후보 자체가 없거나 그 밑에 도구가 없는(매우
/// 드문 경우) fallback 으로 이 머신에서 실측 확인된 keg-only homebrew 경로를 한 번 더 시도한다 —
/// openjdk@17 은 keg-only 라 /opt/homebrew/bin 에 심볼릭 링크가 없다(`ls /opt/homebrew/bin/jarsigner`
/// 실측 결과 없음, `/opt/homebrew/opt/openjdk@17/bin/jarsigner` 는 실재 확인). signing.rs 의 Android
/// release 서명 사후 검증(jarsigner -verify, keytool -list/-printcert)이 이 함수로 도구를 찾는다.
pub fn resolve_jdk_tool(tool_name: &str) -> Option<PathBuf> {
    if let Some(java_home) = resolve_java_home() {
        let candidate = Path::new(&java_home).join("bin").join(tool_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let homebrew_candidate = PathBuf::from(format!("/opt/homebrew/opt/openjdk@17/bin/{tool_name}"));
    if homebrew_candidate.is_file() {
        return Some(homebrew_candidate);
    }
    None
}

/// env_clear() 후 allowlist + PATH(보강) + JAVA_HOME/ANDROID_HOME(fallback 포함) 만 선별 주입한다.
/// stdio 는 호출부 책임(preflight 는 버림, build 는 로그 파일로 연결) — 여기선 env 만 다룬다.
pub fn apply_allowlisted_env(cmd: &mut Command) {
    cmd.env_clear();
    cmd.env("PATH", fixed_path_env());
    for key in ENV_ALLOWLIST {
        if let Ok(value) = std::env::var(key) {
            cmd.env(key, value);
        }
    }
    if let Some(java_home) = resolve_java_home() {
        cmd.env("JAVA_HOME", java_home);
    }
    if let Some(android_home) = resolve_android_home() {
        cmd.env("ANDROID_HOME", &android_home);
        cmd.env("ANDROID_SDK_ROOT", android_home);
    }
}

/// 새 process group 의 리더로 자식을 띄우도록 설정한다(Unix 전용) — flutter/xcodebuild/gradle
/// 래퍼가 만드는 손자 프로세스까지 나중에 한 번에 정리하기 위한 전제(kill_process_group 과 짝,
/// 설계 요구사항). Windows 는 process group 모델 자체가 달라(Job Object 필요) 여기선
/// 시접만 남기고 구현하지 않는다(OS 추상화 원칙 — 이후 Windows 확장 때 별도 구현).
#[cfg(unix)]
pub fn spawn_in_new_process_group(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

#[cfg(not(unix))]
pub fn spawn_in_new_process_group(_cmd: &mut Command) {}

/// spawn_in_new_process_group 으로 띄운 자식의 pgid 전체에 SIGKILL 을 보낸다(pgid == 자기 pid 이므로
/// 이 함수는 그렇게 띄운 자식에만 안전하게 쓸 수 있다). Rust std 에는 killpg 래퍼가 없어 `kill -9
/// -<pid>` 로 POSIX kill(2) 의 "pid 가 음수면 그룹 전체" 규약을 그대로 이용한다 — libc FFI/unsafe 없이
/// 기존 코드베이스 전체가 이미 쓰고 있는 "Command::new + 고정 argv" 패턴만으로 처리한다.
#[cfg(unix)]
pub fn kill_process_group(pid: u32) {
    // 이미 죽은 그룹에 보낼 때 "No such process" 가 표준에러로 나오는 게 정상 경로다 — 화면/로그에
    // 노이즈를 남기지 않도록 버린다(호출부는 성공 여부를 신경 쓰지 않는 best-effort 정리). 바이너리는
    // PATH 검색 대신 `/bin/kill` 절대경로로 고정한다 — GUI 앱은 launchd 기본 PATH 를 물려받아
    // 터미널과 달리 PATH 가 불안정할 수 있다(설계 요구사항).
    let _ = Command::new("/bin/kill")
        .arg("-9")
        .arg(format!("-{pid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(unix))]
pub fn kill_process_group(_pid: u32) {}

/// pid 생존 여부 실측 — `kill -0`는 실제로 신호를 보내지 않고 존재/권한만 확인하는 표준 POSIX 관례다.
/// 여기서도 `/bin/kill` 절대경로로 통일한다(설계 요구사항). 죽은 pid 에 대한 "No such process" 표준에러도 정상 경로라
/// 버린다(위 kill_process_group 과 동일 이유).
#[cfg(unix)]
pub fn is_pid_alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub fn is_pid_alive(_pid: u32) -> bool {
    false
}
