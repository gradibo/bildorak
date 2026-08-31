// bin/cli.rs - bildorak-cli: 빌도락 GUI(src-tauri/src, bin "bildorak")가 쓰는 것과 완전히 같은 앱
// 데이터(paths::base_dir(), 실측: macOS 에선 ~/Library/Application Support/com.gradibo.bildorak)를
// 그대로 읽고 써서, 터미널에서 등록된 Flutter 앱을 빌드/점검한다(3단계, 1차 범위 - 빌드 중심). GUI
// 프로세스와 완전히 독립된 별도 바이너리라 GUI 를 띄우지 않고도 쓸 수 있다(같은 Cargo 패키지의 2번째
// [[bin]] - lib.rs 의 pub mod 선언 덕분에 AppHandle 없이 코어 모듈을 직접 호출한다).
//
// 엔진 원칙(commands.rs 파일 상단 주석과 동일하게 유지): 실제 실행 bin/argv 는 여기서 절대 직접
// 조립하지 않는다 - build::start_build(target: BuildTarget) 하나만 부르고, 실제 flutter 인자는 항상
// build::resolve_command() 의 고정 맵(또는 release 서명/export 주입, build.rs 안에서만)에서 나온다. 이
// 파일은 verbose/--info 류 플래그를 어떤 경로로도 추가하지 않는다(build.rs:990-992 경고 - gradle -P
// 비밀번호가 로그에 평문으로 샌다).
//
// 비밀 값(keystore/keychain 비밀번호 등)은 이 파일 어디에서도 읽거나 출력하지 않는다 - `keys` 는
// signing::load_signing_keys 가 돌려주는 SigningKeyRecord 를 그대로 보여줄 뿐이고, 그 타입 자체가
// 비밀번호를 담지 않는다(model.rs::SigningKeyRecord 문서 참고 - keychain 서비스 "이름"(참조)만 있다).
//
// 종료 코드: 성공 0 / 실패(비0) - Ctrl+C 로 빌드를 취소하면 130(=128+SIGINT, 셸 표준 관례). `status`/
// `doctor` 는 점검 결과에 fail 항목이 하나라도 있으면 1(스크립트에서 `&&` 로 이어 쓸 수 있게).

use bildorak_lib::build::{self, BuildObserver};
use bildorak_lib::model::{
    overall_status_of, BuildJob, BuildJobStatus, BuildStatus, BuildTarget, CheckItem, CheckStatus,
    PreflightRun, ProjectRecord, ReleaseChannel, ReleaseRecord, ReleaseStatus, SigningKeyRecord,
};
use bildorak_lib::{paths, preflight, releases, signing, store};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "bildorak-cli", version, about = "빌도락 CLI - 등록된 Flutter 앱을 터미널에서 빌드/점검해요.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 등록된 앱(Flutter 프로젝트) 목록을 보여줘요.
    Apps {
        /// 구조적 JSON 으로 출력해요(스크립트/파이프용).
        #[arg(long)]
        json: bool,
    },
    /// 등록된 앱을 로컬에서 빌드해요. 완료될 때까지 기다리면서 로그를 그대로 보여줘요.
    Build {
        /// 등록된 앱 이름(GUI 의 pubspec.yaml name 그대로).
        app: String,
        /// 빌드 대상.
        #[arg(long)]
        target: CliTarget,
        /// 완료 후 구조적 JSON 으로 출력해요(진행 중 로그는 표준출력에 흘리지 않아요 - 표준출력은 끝의
        /// JSON 객체 하나만 나가야 스크립트가 안전하게 파이프할 수 있어요).
        #[arg(long)]
        json: bool,
    },
    /// 빌드 준비 점검 결과와 최근 빌드 상태를 보여줘요.
    Status {
        app: String,
        #[arg(long)]
        json: bool,
    },
    /// 등록된 서명키 목록을 보여줘요(비밀번호 값은 어디에도 나오지 않아요).
    Keys {
        #[arg(long)]
        json: bool,
    },
    /// Flutter/Xcode/CocoaPods/Android SDK 등 빌드 환경이 준비됐는지 점검해요.
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// 등록된 앱의 릴리스 기록을 보여줘요(읽기 전용).
    Releases {
        /// 등록된 앱 이름(GUI 의 pubspec.yaml name 그대로).
        app: String,
        #[arg(long)]
        json: bool,
    },
}

/// CLI `--target` 값 ↔ 코어(model::BuildTarget) 매핑 - model.rs::BuildTarget 은 코어(Tauri 참조 0, 설계
/// 문서의 실측 그대로)라 clap 의존을 여기(CLI 전용 어댑터)에서만 붙인다. 네 값 모두 이름을
/// 명시로 고정한다 - model.rs 의 serde snake_case(BuildTarget::as_str())와는 다른 규칙이고, 특히
/// "ios-sim"은 "ios_sim_debug"를 기계적으로 하이픈 변환한 값이 아니라 의도적으로 줄인 별칭이라 clap
/// 기본 케이스 변환에 기대지 않는다.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliTarget {
    #[value(name = "ios-sim")]
    IosSim,
    #[value(name = "android-debug")]
    AndroidDebug,
    #[value(name = "ios-release")]
    IosRelease,
    #[value(name = "android-release")]
    AndroidRelease,
}

impl CliTarget {
    fn to_build_target(self) -> BuildTarget {
        match self {
            CliTarget::IosSim => BuildTarget::IosSimDebug,
            CliTarget::AndroidDebug => BuildTarget::AndroidDebug,
            CliTarget::IosRelease => BuildTarget::IosRelease,
            CliTarget::AndroidRelease => BuildTarget::AndroidRelease,
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        Commands::Apps { json } => cmd_apps(json),
        Commands::Build { app, target, json } => cmd_build(app, target, json),
        Commands::Status { app, json } => cmd_status(app, json),
        Commands::Keys { json } => cmd_keys(json),
        Commands::Doctor { json } => cmd_doctor(json),
        Commands::Releases { app, json } => cmd_releases(app, json),
    };
    std::process::exit(code);
}

// ── 공통 헬퍼 ──────────────────────────────────────────────────────────────

fn resolve_base_dir() -> Result<PathBuf, i32> {
    paths::base_dir().map_err(|e| fail(&e))
}

fn fail(message: &str) -> i32 {
    eprintln!("{message}");
    1
}

/// 이름으로 등록된 프로젝트를 찾는다. 앱 이름은 pubspec.yaml 의 name 필드 그대로라 유일성이 보장되지
/// 않는다 - 서로 다른 경로에 같은 이름의 Flutter 프로젝트를 각각 등록할 수 있다(의도된 동작). 매치가
/// 없으면 Ok(None)(호출부가 app_not_found_message 로 처리), 정확히 1개면 Ok(Some(..)), 2개 이상이면
/// Err(..) - 조용히 첫 번째를 골라 엉뚱한 경로의 앱을 빌드/조회하는 사고를 막는다.
fn find_project(projects: &[ProjectRecord], name: &str) -> Result<Option<ProjectRecord>, String> {
    let matches: Vec<&ProjectRecord> = projects.iter().filter(|p| p.name == name).collect();
    match matches.as_slice() {
        [] => Ok(None),
        [only] => Ok(Some((*only).clone())),
        many => {
            let listed = many
                .iter()
                .map(|p| format!("- {}", p.repo_path))
                .collect::<Vec<_>>()
                .join("\n");
            Err(format!(
                "'{name}' 이름의 앱이 여러 개 등록돼 있어요 - 경로가 다른 동명 앱이 여러 개예요. 어느 걸 \
                 쓸지 특정할 수 없어요.\n{listed}"
            ))
        }
    }
}

fn app_not_found_message(name: &str, projects: &[ProjectRecord]) -> String {
    if projects.is_empty() {
        format!(
            "등록된 앱을 찾지 못했어요: {name} (등록된 앱이 아직 없어요 - 빌도락 GUI 에서 먼저 등록해 주세요.)"
        )
    } else {
        let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
        format!("등록된 앱을 찾지 못했어요: {name}\n등록된 앱: {}", names.join(", "))
    }
}

fn check_status_mark(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Pass => "OK",
        CheckStatus::Warn => "주의",
        CheckStatus::Fail => "실패",
    }
}

fn print_checks(checks: &[CheckItem]) {
    for c in checks {
        println!("[{}] {}: {}", check_status_mark(c.status), c.label, c.message);
        if let Some(next) = &c.next_action {
            println!("      -> {next}");
        }
    }
}

fn build_job_status_label(status: BuildJobStatus) -> &'static str {
    match status {
        BuildJobStatus::Running => "진행 중",
        BuildJobStatus::Success => "성공",
        BuildJobStatus::Failed => "실패",
    }
}

/// types.ts::RELEASE_STATUS_LABEL 과 같은 문구 - cmd_releases 사람용 출력이 {:?}(PascalCase Debug) 대신
/// 이 한국어 라벨을 쓴다(build_job_status_label 선례).
fn release_status_label(status: ReleaseStatus) -> &'static str {
    match status {
        ReleaseStatus::Preparing => "준비 중",
        ReleaseStatus::Submitted => "심사 제출",
        ReleaseStatus::Approved => "승인됨",
        ReleaseStatus::Rejected => "반려",
        ReleaseStatus::Released => "출시됨",
    }
}

/// types.ts::RELEASE_CHANNEL_LABEL 과 같은 문구 - release_status_label 과 동일 목적.
fn release_channel_label(channel: ReleaseChannel) -> &'static str {
    match channel {
        ReleaseChannel::AppStore => "App Store",
        ReleaseChannel::PlayStore => "Play Store",
        ReleaseChannel::Github => "GitHub",
        ReleaseChannel::Other => "기타",
    }
}

// ── apps ───────────────────────────────────────────────────────────────────

fn cmd_apps(json: bool) -> i32 {
    let base_dir = match resolve_base_dir() {
        Ok(d) => d,
        Err(code) => return code,
    };
    let projects = match store::load_projects_from_dir(&base_dir) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&projects).unwrap_or_default());
    } else if projects.is_empty() {
        println!("등록된 앱이 없어요. 빌도락 GUI 에서 먼저 프로젝트를 등록해 주세요.");
    } else {
        for p in &projects {
            println!("{}\t{}", p.name, p.repo_path);
        }
    }
    0
}

// ── status ─────────────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
struct StatusOutput {
    preflight: PreflightRun,
    build: BuildStatus,
}

fn cmd_status(app: String, json: bool) -> i32 {
    let base_dir = match resolve_base_dir() {
        Ok(d) => d,
        Err(code) => return code,
    };
    let projects = match store::load_projects_from_dir(&base_dir) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let project = match find_project(&projects, &app) {
        Ok(Some(p)) => p,
        Ok(None) => return fail(&app_not_found_message(&app, &projects)),
        Err(e) => return fail(&e),
    };

    let preflight_run = preflight::run(&project);
    let build_status = match build::get_build_status(&base_dir, &project) {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let overall = preflight_run.overall_status;

    if json {
        let out = StatusOutput { preflight: preflight_run, build: build_status };
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    } else {
        println!("=== {} 빌드 준비 점검 ===", project.name);
        print_checks(&preflight_run.checks);
        println!();
        match &build_status.job {
            Some(job) => {
                println!("최근 빌드: {} ({})", build_job_status_label(job.status), job.target_label);
                if let Some(note) = &job.status_note {
                    println!("  {note}");
                }
                if let (Some(rel), Some(exists)) = (&build_status.artifact_relpath, build_status.artifact_exists) {
                    println!("  산출물: {rel} ({})", if exists { "있음" } else { "없음" });
                }
            }
            None => println!("최근 빌드 기록이 없어요."),
        }
    }

    if overall == CheckStatus::Fail {
        1
    } else {
        0
    }
}

// ── keys ───────────────────────────────────────────────────────────────────

fn cmd_keys(json: bool) -> i32 {
    let base_dir = match resolve_base_dir() {
        Ok(d) => d,
        Err(code) => return code,
    };
    let keys: Vec<SigningKeyRecord> = match signing::load_signing_keys(&base_dir) {
        Ok(k) => k,
        Err(e) => return fail(&e),
    };
    if json {
        // SigningKeyRecord 는 어떤 필드에도 비밀 값을 담지 않는다(model.rs 문서 - keychain 서비스
        // "이름"(참조)만 있을 뿐 비밀번호 원문은 없다) - 그대로 직렬화해도 안전하다.
        println!("{}", serde_json::to_string_pretty(&keys).unwrap_or_default());
    } else if keys.is_empty() {
        println!("등록된 서명키가 없어요.");
    } else {
        for k in &keys {
            let expiry = k.expires_at.as_deref().unwrap_or("확인 불가");
            println!(
                "{}\t{:?}\t만료: {}\t연결된 앱 {}개",
                k.display_name,
                k.kind,
                expiry,
                k.linked_project_ids.len()
            );
        }
    }
    0
}

// ── doctor ─────────────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
struct DoctorOutput {
    overall: CheckStatus,
    checks: Vec<CheckItem>,
}

fn cmd_doctor(json: bool) -> i32 {
    let base_dir = match resolve_base_dir() {
        Ok(d) => d,
        Err(code) => return code,
    };
    let checks = preflight::check_environment(&base_dir);
    let overall = overall_status_of(&checks);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&DoctorOutput { overall, checks }).unwrap_or_default()
        );
    } else {
        print_checks(&checks);
    }
    if overall == CheckStatus::Fail {
        1
    } else {
        0
    }
}

// ── releases ───────────────────────────────────────────────────────────────

fn cmd_releases(app: String, json: bool) -> i32 {
    let base_dir = match resolve_base_dir() {
        Ok(d) => d,
        Err(code) => return code,
    };
    let projects = match store::load_projects_from_dir(&base_dir) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let project = match find_project(&projects, &app) {
        Ok(Some(p)) => p,
        Ok(None) => return fail(&app_not_found_message(&app, &projects)),
        Err(e) => return fail(&e),
    };

    let mut app_releases: Vec<ReleaseRecord> = match releases::load_releases_from_dir(&base_dir) {
        Ok(r) => r.into_iter().filter(|r| r.project_id == project.id).collect(),
        Err(e) => return fail(&e),
    };
    // GUI(commands.rs::list_releases)와 동일하게 최신순(created_at desc)으로 보여준다.
    app_releases.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    if json {
        println!("{}", serde_json::to_string_pretty(&app_releases).unwrap_or_default());
    } else if app_releases.is_empty() {
        println!("등록된 릴리스 기록이 없어요.");
    } else {
        for r in &app_releases {
            let build_label = r.build_number.as_deref().unwrap_or("-");
            println!(
                "{}\t빌드 {}\t{}\t{}\t{}",
                r.version,
                build_label,
                release_status_label(r.status),
                release_channel_label(r.channel),
                r.created_at
            );
        }
    }
    0
}

// ── build ──────────────────────────────────────────────────────────────────

/// 사람용 출력 - 새 로그 줄을 그대로 표준출력에 흘려보낸다(터미널에서 `flutter build` 를 직접 돌리는
/// 것과 비슷한 체감을 준다).
struct HumanLogObserver;
impl BuildObserver for HumanLogObserver {
    fn on_log(&mut self, lines: &[String]) {
        for line in lines {
            println!("{line}");
        }
    }
    fn on_done(&mut self, _job: &BuildJob) {}
}

/// --json 모드 - 로그를 표준출력에 흘리지 않는다("--json이면 수집 후 최종 JSON" 설계 그대로 - 표준
/// 출력은 끝에 JSON 객체 하나만 나가야 스크립트가 안전하게 파이프할 수 있다).
struct SilentObserver;
impl BuildObserver for SilentObserver {
    fn on_log(&mut self, _lines: &[String]) {}
    fn on_done(&mut self, _job: &BuildJob) {}
}

/// `is_running` 이 false 가 되거나 max_wait 이 지날 때까지 interval 간격으로 짧게 기다린다 -
/// SIGINT 취소 직후 monitor 스레드가 build_jobs.json 에 최종 상태를 다 쓸 시간을 준다. 순수 로직만
/// 분리해서(상태 조회는 클로저로 주입) 실제 job/프로세스 없이도 테스트한다. 타임아웃에 도달해도 에러
/// 없이 그냥 반환한다 - 못 벗어나는 극단적인 경우엔 기존 reconcile_stale_job(build.rs)이 다음 조회 때
/// 안전망으로 정리해 준다.
fn wait_until_not_running<F: FnMut() -> bool>(max_wait: Duration, interval: Duration, mut is_running: F) {
    let deadline = Instant::now() + max_wait;
    while is_running() {
        if Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(interval);
    }
}

fn cmd_build(app: String, target: CliTarget, json: bool) -> i32 {
    let base_dir = match resolve_base_dir() {
        Ok(d) => d,
        Err(code) => return code,
    };
    let projects = match store::load_projects_from_dir(&base_dir) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let project = match find_project(&projects, &app) {
        Ok(Some(p)) => p,
        Ok(None) => return fail(&app_not_found_message(&app, &projects)),
        Err(e) => return fail(&e),
    };
    let build_target = target.to_build_target();

    let job = match build::start_build(&base_dir, &project, build_target) {
        Ok(j) => j,
        Err(e) => return fail(&e),
    };

    if job.status != BuildJobStatus::Running {
        // start_build 는 spawn 자체가 즉시 실패한 경우(예: 이미 진행 중, 프로젝트 폴더 없음)도
        // Ok(job)(status=Failed)로 돌려준다(build.rs::spawn_build_job 문서 참고) - 여기서 바로 결과를
        // 보여주고 실패로 끝낸다.
        print_final_build_status(&base_dir, &project, json);
        return 1;
    }

    eprintln!(
        "빌드를 시작했어요 ({}). 완료까지 기다립니다... (Ctrl+C 로 취소)",
        build::target_label(build_target)
    );

    // SIGINT(Ctrl+C) → 기존 cancel_build 로 정상 취소 후 비0 종료(130 = 128+SIGINT, 셸 표준 관례).
    // ctrlc 크레이트는 실제 시그널 컨텍스트가 아니라 전용 스레드에서 핸들러를 실행하므로, 여기서 파일
    // IO/kill_process_group 같은 블로킹 작업을 해도 안전하다.
    {
        let cancel_base_dir = base_dir.clone();
        let cancel_project = project.clone();
        if ctrlc::set_handler(move || {
            eprintln!("\n취소 요청을 받았어요. 빌드를 중단할게요...");
            let _ = build::cancel_build(&cancel_base_dir, &cancel_project);
            // cancel_build 는 kill 신호만 보내고 바로 돌아온다 - build_jobs.json 에 최종 상태(Failed)를
            // 실제로 쓰는 건 spawn_build_job 이 띄운 monitor 스레드(비동기, try_wait 500ms 폴링)다. 여기서
            // 바로 exit(130) 하면 프로세스 전체가 그 자리에서 죽어 monitor 스레드가 파일을 못 쓴 채 끊기고,
            // job 이 Running 으로 남는다 - 나중에 reconcile_stale_job 이 이걸 stale 로 판정해 "비정상
            // 종료된 것으로 보여요" 라는 부정확한 문구로 덮어쓴다(정상 취소인데 에러처럼 보인다). 그래서
            // exit 전에 monitor 스레드가 파일을 다 쓸 때까지 최대 5초, 250ms 간격으로 짧게 기다린다.
            wait_until_not_running(Duration::from_secs(5), Duration::from_millis(250), || {
                build::get_build_status(&cancel_base_dir, &cancel_project)
                    .map(|status| {
                        status.job.map(|j| j.status == BuildJobStatus::Running).unwrap_or(false)
                    })
                    .unwrap_or(false)
            });
            std::process::exit(130);
        })
        .is_err()
        {
            eprintln!(
                "경고: Ctrl+C 핸들러를 등록하지 못했어요 - 취소는 여전히 GUI 또는 다른 터미널에서 가능해요."
            );
        }
    }

    let mut human_observer = HumanLogObserver;
    let mut silent_observer = SilentObserver;
    let observer: &mut dyn BuildObserver = if json { &mut silent_observer } else { &mut human_observer };

    let final_job = match build::watch_build_to_completion(
        &base_dir,
        &project,
        build_target,
        observer,
        Duration::from_millis(700),
    ) {
        Ok(j) => j,
        Err(e) => return fail(&e),
    };

    print_final_build_status(&base_dir, &project, json);
    if final_job.status == BuildJobStatus::Success {
        0
    } else {
        1
    }
}

/// build::get_build_status 를 다시 한번 불러 최종 결과(job + 로그 tail + 산출물 확인)를 출력한다 -
/// --json 은 BuildStatus 를 그대로, 사람용은 상태/메모/산출물 존재 여부만 요약한다.
fn print_final_build_status(base_dir: &Path, project: &ProjectRecord, json: bool) {
    let status = match build::get_build_status(base_dir, project) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("빌드 상태를 확인하지 못했어요: {e}");
            return;
        }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&status).unwrap_or_default());
        return;
    }
    let Some(job) = &status.job else {
        println!("빌드 상태를 확인하지 못했어요.");
        return;
    };
    println!("빌드 결과: {}", build_job_status_label(job.status));
    if let Some(note) = &job.status_note {
        println!("{note}");
    }
    if let (Some(rel), Some(exists)) = (&status.artifact_relpath, status.artifact_exists) {
        println!("산출물: {rel} ({})", if exists { "있음" } else { "없음" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bildorak_lib::model::Platform;
    use std::cell::Cell;

    fn fake_project(name: &str, repo_path: &str) -> ProjectRecord {
        ProjectRecord {
            id: format!("test-{name}-{repo_path}"),
            name: name.to_string(),
            selected_path: repo_path.to_string(),
            repo_path: repo_path.to_string(),
            version: None,
            build_number: None,
            platforms: vec![Platform::Ios],
            registered_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    // ── find_project ──────────────────────────────────────────────────────

    #[test]
    fn find_project_returns_none_when_no_match() {
        let projects = vec![fake_project("myapp", "/repo/a")];
        assert!(find_project(&projects, "other").unwrap().is_none());
    }

    #[test]
    fn find_project_returns_the_single_match() {
        let projects = vec![fake_project("myapp", "/repo/a"), fake_project("otherapp", "/repo/b")];
        let found = find_project(&projects, "myapp").unwrap().expect("정확히 1개 매치해야 한다");
        assert_eq!(found.repo_path, "/repo/a");
    }

    #[test]
    fn find_project_errors_with_all_repo_paths_when_name_is_ambiguous() {
        let projects =
            vec![fake_project("myapp", "/repo/a"), fake_project("myapp", "/repo/b"), fake_project("other", "/repo/c")];
        let err = find_project(&projects, "myapp").expect_err("동명 앱이 2개면 에러여야 한다");
        assert!(err.contains("경로가 다른 동명 앱이 여러 개예요"), "실제 메시지: {err}");
        assert!(err.contains("/repo/a"), "실제 메시지: {err}");
        assert!(err.contains("/repo/b"), "실제 메시지: {err}");
        assert!(!err.contains("/repo/c"), "매치 안 된 프로젝트 경로는 나오면 안 된다: {err}");
    }

    // ── wait_until_not_running ────────────────────────────────────────────

    #[test]
    fn wait_until_not_running_returns_immediately_when_already_stopped() {
        let start = Instant::now();
        wait_until_not_running(Duration::from_secs(5), Duration::from_millis(250), || false);
        assert!(start.elapsed() < Duration::from_millis(100), "이미 멈춘 상태면 곧바로 반환해야 한다");
    }

    #[test]
    fn wait_until_not_running_stops_as_soon_as_condition_flips() {
        let calls = Cell::new(0u32);
        let start = Instant::now();
        wait_until_not_running(Duration::from_secs(5), Duration::from_millis(10), || {
            calls.set(calls.get() + 1);
            calls.get() < 3 // 처음 두 번은 running=true, 세 번째 호출부터 false
        });
        assert_eq!(calls.get(), 3);
        assert!(start.elapsed() < Duration::from_millis(500), "조건이 풀리면 max_wait 까지 기다리면 안 된다");
    }

    #[test]
    fn wait_until_not_running_gives_up_after_max_wait() {
        let start = Instant::now();
        wait_until_not_running(Duration::from_millis(60), Duration::from_millis(20), || true);
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(60), "max_wait 이전에 포기하면 안 된다: {elapsed:?}");
        assert!(elapsed < Duration::from_millis(500), "max_wait 을 크게 초과하면 안 된다: {elapsed:?}");
    }
}
