// paths.rs — CLI(bildorak-cli, src/bin/cli.rs)가 GUI(store.rs)와 같은 app config dir 을 계산하기 위한
// additive 헬퍼. GUI 는 여전히 store.rs::app_config_dir(app: &AppHandle)(Tauri path API)를 그대로 쓴다 —
// 이 파일은 AppHandle 이 없는 CLI 바이너리가 "GUI 와 똑같은 폴더"를 스스로 계산할 수 있게 해 줄 뿐,
// GUI 쪽 코드는 이 파일을 전혀 참조하지 않는다(무회귀).
//
// 실측: Tauri v2 의 app.path().app_config_dir() 는 내부적으로 dirs::config_dir()/<identifier> 를 쓴다
// (store.rs:12-14 주석이 이미 같은 사실을 확증) — macOS 에선 dirs 6.0.0(Cargo.lock 에 이미 고정된
// 버전, Tauri 가 물고 온다)의 config_dir() 이 "$HOME/Library/Application Support" 를 반환해 최종 경로는
// "~/Library/Application Support/com.gradibo.bildorak" 이 된다. 이 머신에서
// `ls ~/Library/Application\ Support/com.gradibo.bildorak` 로 실제 GUI 데이터(projects.json 등)가 그
// 경로에 있는 것도 확인했다. 아래 tests 모듈이 이 등가성을 리터럴 경로와 비교해 고정해 둔다 — dirs나
// Tauri 버전이 바뀌어 계산 방식이 달라지면 테스트가 즉시 깨진다(drift 차단).

use std::path::PathBuf;

/// tauri.conf.json 의 identifier(무변경) — GUI/CLI 양쪽이 반드시 같은 값을 써야 같은 base_dir 을
/// 가리킨다. 여기 리터럴로 중복해 둔 이유: tauri.conf.json 은 GUI 크레이트 빌드 시점에만 tauri_build
/// 코드젠으로 읽히고, CLI(별도 bin 크레이트)에서 그 값을 다시 꺼내올 손쉬운 방법이 없다 — 값이
/// 바뀌면 아래 base_dir_matches_macos_app_support_literal 테스트가 잡아준다.
pub const IDENTIFIER: &str = "com.gradibo.bildorak";

/// 폴더 존재를 보장하지 않는 순수 계산 — 테스트에서 실제 사용자의 앱 데이터 폴더에 부수효과(mkdir)를
/// 남기지 않기 위해 base_dir() 과 분리해 둔다.
fn identifier_dir() -> Result<PathBuf, String> {
    dirs::config_dir()
        .map(|dir| dir.join(IDENTIFIER))
        .ok_or_else(|| "설정 폴더 위치를 확인하지 못했어요(HOME 환경변수를 확인해 주세요).".to_string())
}

/// CLI 전용 base_dir — GUI 의 store::app_config_dir(app) 과 반드시 같은 경로를 가리켜야 한다(같은
/// projects.json/settings.json/signing_keys.json/build_jobs.json 을 읽고 써야 CLI 로 GUI 데이터를 그대로
/// 볼 수 있다). GUI 쪽과 동일하게 폴더가 없으면 만든다(store.rs::app_config_dir 과 동일 규칙 — GUI를
/// 한 번도 실행하지 않은 상태에서 CLI 를 먼저 써도 동작해야 한다).
pub fn base_dir() -> Result<PathBuf, String> {
    let dir = identifier_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("설정 폴더를 만들지 못했어요: {e}"))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GUI(store.rs::app_config_dir, Tauri app_config_dir API)가 실제로 귀결되는 물리 경로와 이 함수가
    /// 계산한 값이 같은지 리터럴 경로 기준으로 고정한다(위 파일 상단 "실측" 문단 — drift 차단). macOS
    /// 전용 — 이 값 자체가 macOS 의 dirs::config_dir() 규칙에 근거한 실측이라 다른 OS 에는 적용되지
    /// 않는다(store.rs::keystore_vault_dir 의 cfg(unix) 가드와 같은 이유).
    #[test]
    #[cfg(target_os = "macos")]
    fn base_dir_matches_macos_app_support_literal() {
        let home = std::env::var("HOME").expect("테스트 환경에 HOME 이 있어야 한다");
        let expected = PathBuf::from(home).join("Library/Application Support").join(IDENTIFIER);
        assert_eq!(identifier_dir().expect("계산이 실패하면 안 된다"), expected);
    }
}
