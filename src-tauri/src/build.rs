// build.rs - 로컬 빌드 실행(2차). 프론트는 (project_id, target enum) 만 invoke - 실제 bin/args 는
// resolve_command() 의 고정 맵에서만 나온다(엔진 원칙). Command::new(bin).args(args) 로만 실행하고
// 문자열 조립/셸 실행은 하지 않는다.
//
// job 라이프사이클: 프로젝트당 진행 중 빌드는 최대 1개(다른 프로젝트는 동시 진행 허용) - 이미 진행
// 중이면 에러 반환(HTTP 409 Conflict 상당). 상태/로그는 base_dir(commands.rs 가 app_config_dir 로
// 넘겨줌) 하위에 저장해 앱 재시작 후에도 마지막 결과를 보여준다. 저장된 job 이 running 인데 pid 가
// 죽어 있으면(비정상 종료) failed 로 정리한다(reconcile_stale_job 이 담당).
//
// 자식은 새 process group 으로 띄운다(child_env::spawn_in_new_process_group) - 앱 종료 시
// kill_all_running_builds() 가 그룹 전체를 정리해 flutter 래퍼의 손자 프로세스(xcodebuild/gradle 등)가
// 고아로 남지 않게 한다(설계 요구사항과 같은 원칙, 대상은 preflight 대신 실제 빌드).
//
// 테스트 용이성을 위해 핵심 로직은 AppHandle 이 아니라 base_dir: &Path 를 받는다(commands.rs 가
// app.path().app_config_dir() 를 resolve 해서 넘겨준다) - Tauri 앱을 띄우지 않고도 #[test] 에서 실제
// job 파이프라인 전체(spawn → 로그 파일 → 상태 파일 → 완료 감지)를 그대로 실행해 검증할 수 있다.
//
// 3차(설계 요구사항) 보강: build_jobs.json 읽기-수정-쓰기 전체를 BUILD_JOBS_FILE_LOCK 하나로
// 직렬화(#1), reconcile 시 pid 재사용을 경과시간으로 방어(#2), 파일 파싱 실패는 self-heal(#3), 완료
// 감지 스레드를 blocking wait() 대신 폴링으로 바꿔 타임아웃 + cancel_build 취소를 모두 처리(#5).
//
// 2단계(무료 오픈소스, 게이트 없음) 추가: 빌드가 끝날 때마다(finalize_build_job) 그 스냅샷을
// build_jobs.json 과는 완전히 별도인 build_history.json 에 best-effort 로 append 한다(get_build_history).
// build_jobs.json/save_single_job/finalize_build_job 의 기존 동작(락 범위·저장 순서·반환값)은 바뀌지
// 않았다 - finalize_build_job 은 기존 블록을 그대로 감싸 결과만 클론해 락 밖에서 히스토리에 추가한다.
// 알림(빌드 완료 macOS 알림)은 AppHandle 이 필요해 이 파일에 두지 않고 commands.rs 가 담당한다 - 이
// 파일은 계속 AppHandle 을 모르는 경계를 유지한다(위 "테스트 용이성" 문단과 동일 이유).
//
// iOS 릴리스 export(설계 결정, 2026-08) 추가: IosRelease 는 이제 항상 flutter build ipa --release 뒤에
// --export-options-plist 를 붙인다(실측: `flutter build ipa --help`, flutter 3.41.8 - --export-method 와
// --export-options-plist 는 동시 사용 불가, flutter_tools build_ios.dart 실측으로 확인됨). plist 안의
// team_id 는 resolve_ios_team_id() 가 프로젝트 project.pbxproj → keychain 배포 인증서 순으로 확정하고,
// 매 빌드 시작 시 base_dir(app config dir) 하위에 새로 써서 넘긴다 - 프로젝트 폴더(git 레포)는 절대
// 건드리지 않는다(write_ios_export_options). flutter build ipa 는 xcodebuild -exportArchive 가 실패해도
// (서명/팀 불일치 등) 자체 exit code 는 0(성공)을 낼 수 있다는 게 flutter_tools 실측으로도 확인돼
// (FlutterCommandResult.success() 를 그대로 반환) - AndroidRelease 서명 검증과 같은 원칙으로 빌드 완료
// 스레드에서 실제 .ipa 파일 존재까지 다시 확인한다(ipa_dir_contains_ipa).

use crate::child_env;
use crate::model::{
    BuildJob, BuildJobStatus, BuildStatus, BuildTarget, CliCommandDoc, ProjectRecord, SigningKeyKind,
};
use crate::signing;
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use uuid::Uuid;

const BUILD_JOBS_FILE: &str = "build_jobs.json";
/// build_jobs.json(현재 상태, 프로젝트당 1건)과 완전히 별도인 파일 - 완료된 빌드 스냅샷을 쌓아 두는
/// 히스토리 전용 저장소(2단계, "로컬 편의 기능", 무료 오픈소스). 이 파일을 읽고 쓰는
/// 어떤 코드도 build_jobs.json 의 기존 로직(락/self-heal/reconcile)을 건드리지 않는다.
const BUILD_HISTORY_FILE: &str = "build_history.json";
/// 프로젝트당 히스토리 보관 개수 - 넘으면 오래된 것부터 제거한다(편의 기능이라 무한정 쌓아두지 않음).
const BUILD_HISTORY_MAX_PER_PROJECT: usize = 20;
const BUILD_LOGS_DIR: &str = "build-logs";
/// tail 계산 시 파일 끝에서 최대 이만큼만 읽는다 - 로그가 아무리 길어도 메모리 안전하다.
const LOG_TAIL_MAX_BYTES: u64 = 200_000;
/// 빌드가 이보다 오래 응답이 없으면(프로세스가 안 끝남) 자동으로 중단시킨다 - 무한 hang 시 앱을 강제
/// 종료하는 것 말고는 탈출구가 없던 상태를 해소한다(설계 요구사항). 로컬 디버그 빌드
/// 기준으로 넉넉하게 잡았다 - 최초 pod install/gradle daemon 콜드스타트를 포함해도 정상적으로는
/// 이보다 짧게 끝난다. 이걸 넘기면 사실상 멈춘 것으로 본다.
const BUILD_TIMEOUT: Duration = Duration::from_secs(45 * 60);
/// reconcile 시 pid 가 살아있어도 시작 시각 기준 이보다 오래된 running 기록은 무조건 stale 로 본다 -
/// pid 재사용 오판 방어(설계 요구사항, reconcile_stale_job 참고).
const RECONCILE_MAX_AGE_HOURS: i64 = 3;

type BuildJobMap = HashMap<String, BuildJob>;
/// project_id → 완료된 빌드 스냅샷 목록(최신이 앞) - build_history.json 의 저장 형태.
type BuildHistoryMap = HashMap<String, Vec<BuildJob>>;

/// 프로젝트당 고정 커맨드 맵 - target 이 곧 앱 전체에 닫힌 집합이라 앱 id 별로 나눌 필요가 없다
/// (bildorak 은 임의로 등록된 Flutter 프로젝트 전체에 공통 적용).
pub fn resolve_command(target: BuildTarget) -> (&'static str, Vec<&'static str>) {
    match target {
        BuildTarget::IosSimDebug => ("flutter", vec!["build", "ios", "--simulator", "--debug"]),
        BuildTarget::AndroidDebug => ("flutter", vec!["build", "apk", "--debug"]),
        BuildTarget::IosRelease => ("flutter", vec!["build", "ipa", "--release"]),
        BuildTarget::AndroidRelease => ("flutter", vec!["build", "appbundle", "--release"]),
    }
}

/// 성공 시 실제로 존재하는지 확인할 고정 산출물 경로(project.repoPath 기준 상대경로).
///
/// AndroidRelease 는 파일 하나(app-release.aab)로 고정된다 - flutter_tools 실측(flutter 3.41.8,
/// packages/flutter_tools/lib/src/android/gradle.dart::getBundleDirectory/findBundleFile)으로
/// 확인: non-flavor 릴리스 빌드는 항상 `build/app/outputs/bundle/release/app-release.aab` 하나만
/// 만든다(모듈 프로젝트는 예외지만 bildorak 은 pubspec.yaml 이 있는 일반 앱 프로젝트만 등록 대상).
///
/// IosRelease 는 파일이 아니라 **디렉터리**를 가리킨다 - flutter_tools 실측
/// (packages/flutter_tools/lib/src/ios/application_package.dart::ipaOutputPath =
/// getIosBuildDirectory()/ipa)으로 확인: 실제 .ipa 파일명은 앱 이름/스킴에 따라 프로젝트마다 달라
/// 고정 파일명을 쓸 수 없다(사전 우려 그대로, 와일드카드는 이 함수의 반환 타입(고정
/// &'static str, resolve_command 와 동일한 "고정 맵" 원칙)으로 표현할 수도 없다) - 그래서 디렉터리
/// 자체의 존재 여부로 판정한다(get_build_status 의 Path::join(rel).exists() 가 디렉터리에도 그대로
/// 동작). ⚠️ 실측으로 추가 확인된 것: `flutter build ipa` 는 xcodebuild -exportArchive 단계가
/// 실패해도(서명/프로비저닝 문제 등) flutter 커맨드 자체는 exit code 0(성공)을 낼 수 있다(archive는
/// 됐으니 "성공"으로 침) - 즉 job.status == success 인데 이 디렉터리가 없는 경우가 정상적으로
/// 발생할 수 있다. 새로 만들 필요 없이 이미 있는 안전장치로 충분하다: get_build_status 가 그 자리에서
/// artifact_exists 를 다시 확인하고, artifactStatusLine(copy.ts)이 success + exists == false 조합을
/// "산출물 확인 필요"로 정확히 안내한다.
pub fn expected_artifact_relpath(target: BuildTarget) -> &'static str {
    match target {
        BuildTarget::IosSimDebug => "build/ios/iphonesimulator/Runner.app",
        BuildTarget::AndroidDebug => "build/app/outputs/flutter-apk/app-debug.apk",
        BuildTarget::IosRelease => "build/ios/ipa",
        BuildTarget::AndroidRelease => "build/app/outputs/bundle/release/app-release.aab",
    }
}

/// job 시작 시점에 BuildJob.targetLabel 로 스냅샷 저장할 비개발자 톤 라벨.
pub fn target_label(target: BuildTarget) -> &'static str {
    match target {
        BuildTarget::IosSimDebug => "iOS 시뮬레이터 디버그 빌드",
        BuildTarget::AndroidDebug => "Android 디버그 빌드",
        BuildTarget::IosRelease => "iOS 릴리스(ipa)",
        BuildTarget::AndroidRelease => "Android 릴리스(aab)",
    }
}

fn jobs_file_path(base_dir: &Path) -> PathBuf {
    base_dir.join(BUILD_JOBS_FILE)
}

fn history_file_path(base_dir: &Path) -> PathBuf {
    base_dir.join(BUILD_HISTORY_FILE)
}

fn logs_dir_path(base_dir: &Path) -> PathBuf {
    base_dir.join(BUILD_LOGS_DIR)
}

/// CLI(bildorak-cli) 가 바이트 오프셋 tail(아래 read_log_from_offset)에 쓸 경로를 직접 계산할 수 있게
/// pub 노출한다(3단계, additive) - get_build_status 의 50줄 tail 은 화면 카드용으로 충분하지만, CLI
/// `build` 는 빌드 내내 stdout 으로 로그 전량을 흘려보내야 해서 파일 경로를 직접 알아야 한다. 계산
/// 규칙 자체는 전혀 바뀌지 않았다.
pub fn log_file_path(base_dir: &Path, project_id: &str, target: BuildTarget) -> PathBuf {
    logs_dir_path(base_dir).join(format!("{project_id}-{}.log", target.as_str()))
}

/// 저장된 job 목록을 읽는다. 파일이 없으면(첫 빌드 전) 빈 목록. 파싱에 실패하면(JSON 손상) 예전엔
/// 하드 에러라 빌드 기능 전체가 막혔다 - 이제는 손상 파일을 `.corrupt-<타임스탬프>` 로 백업해 두고
/// 빈 목록으로 self-heal 한다(설계 요구사항). 백업은 지우지 않는다 - 나중에 원인 조사가
/// 필요할 수 있다. 백업 자체가 실패해도(권한 문제 등) self-heal 은 계속 진행한다 - 빌드 기능이 막히는
/// 것보다 백업 하나 못 남기는 쪽이 낫다.
fn load_build_jobs(base_dir: &Path) -> Result<BuildJobMap, String> {
    let path = jobs_file_path(base_dir);
    if !path.exists() {
        return Ok(BuildJobMap::new());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("빌드 상태를 읽지 못했어요: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(BuildJobMap::new());
    }
    match serde_json::from_str(&raw) {
        Ok(jobs) => Ok(jobs),
        Err(_) => {
            let backup_path = base_dir.join(format!(
                "{BUILD_JOBS_FILE}.corrupt-{}",
                Utc::now().format("%Y%m%d%H%M%S")
            ));
            let _ = fs::rename(&path, &backup_path);
            Ok(BuildJobMap::new())
        }
    }
}

/// store.rs 의 write_json_atomic(temp + rename) 을 그대로 재사용한다(설계 요구사항).
fn save_build_jobs(base_dir: &Path, jobs: &BuildJobMap) -> Result<(), String> {
    let path = jobs_file_path(base_dir);
    let raw = serde_json::to_string_pretty(jobs)
        .map_err(|e| format!("저장할 데이터를 만들지 못했어요: {e}"))?;
    crate::store::write_json_atomic(&path, &raw)
        .map_err(|e| format!("빌드 상태를 저장하지 못했어요: {e}"))
}

/// 저장된 빌드 히스토리를 읽는다(2단계) - build_jobs.json 과 완전히 다른 파일이라 이 함수가 실패해도
/// 현재 빌드 상태(build_jobs.json)에는 전혀 영향이 없다. 파일이 없으면(아직 완료된 빌드가 없거나 이
/// 기능 이전 버전) 빈 목록. 파싱 실패 시 load_build_jobs 와 동일한 self-heal 규칙(손상 파일 백업 후
/// 빈 목록으로 계속 진행)을 그대로 따른다.
fn load_build_history(base_dir: &Path) -> Result<BuildHistoryMap, String> {
    let path = history_file_path(base_dir);
    if !path.exists() {
        return Ok(BuildHistoryMap::new());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("빌드 히스토리를 읽지 못했어요: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(BuildHistoryMap::new());
    }
    match serde_json::from_str(&raw) {
        Ok(history) => Ok(history),
        Err(_) => {
            let backup_path = base_dir.join(format!(
                "{BUILD_HISTORY_FILE}.corrupt-{}",
                Utc::now().format("%Y%m%d%H%M%S")
            ));
            let _ = fs::rename(&path, &backup_path);
            Ok(BuildHistoryMap::new())
        }
    }
}

/// write_json_atomic 재사용(build_jobs.json 저장과 동일 방식) - temp + rename 이라 저장 도중 죽어도
/// 히스토리 파일이 반쯤 쓰인 내용으로 손상되지 않는다.
fn save_build_history(base_dir: &Path, history: &BuildHistoryMap) -> Result<(), String> {
    let path = history_file_path(base_dir);
    let raw = serde_json::to_string_pretty(history)
        .map_err(|e| format!("저장할 히스토리 데이터를 만들지 못했어요: {e}"))?;
    crate::store::write_json_atomic(&path, &raw)
        .map_err(|e| format!("빌드 히스토리를 저장하지 못했어요: {e}"))
}

/// 저장된 job 이 running 인데 pid 가 이미 죽어 있으면(비정상 종료로 close 콜백을 못 받은 경우) failed
/// 로 강제 전환한다.
///
/// 이번 프로세스가 이 project_id 의 완료 감지 스레드를 이미 갖고 있으면(running_builds 에 등록돼
/// 있으면 = 우리가 이번 세션에서 직접 스폰한 빌드) 그 스레드가 child.wait() 로 진짜 종료 코드를 곧
/// finalize_build_job 에 기록할 것이므로, 여기서 pid 생존만 보고 섣불리 stale 로 단정하지 않는다.
/// (그렇게 하면 명령이 아주 빨리 끝나는 경우 - 자식이 exit 했지만 우리 스레드가 아직 wait() 을
/// 호출하기 전인 짧은 틈 - 에 진짜 종료 코드를 "확인 불가"로 덮어써 버리는 race 가 생긴다. 실제로
/// 이 문제 때문에 fast_fake_command_* 테스트가 처음에 깨졌다.) reconcile 은 재시작 후 완료 감지
/// 스레드 자체가 없는 job 에만 의미가 있다 - "재시작 시" 케이스 그대로.
fn reconcile_stale_job(base_dir: &Path, job: BuildJob) -> Result<BuildJob, String> {
    if job.status != BuildJobStatus::Running {
        return Ok(job);
    }
    let has_local_watcher = running_builds()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(&job.project_id);
    if has_local_watcher {
        return Ok(job);
    }
    // pid 가 살아있어도(kill -0) 재시작 사이에 OS 가 그 번호를 완전히 다른 프로세스에 재사용했을 수
    // 있다(설계 요구사항). `ps -o comm=` 명령어 이름 대조는 검토했지만 채택하지 않았다 -
    // flutter 는 bash 래퍼 스크립트라(`file $(which flutter)` 실측 결과 "Bourne-Again shell script")
    // 실행 도중 dart 로 exec 될 수 있어 comm 이 "flutter" 로 안 잡히는 경우가 흔하고, 잘못 매칭하면
    // 아직 실제로 도는 빌드를 stale 로 오판해 "프로젝트당 최대 1개 동시 빌드" 제약을 오히려 깨뜨릴
    // 위험이 있다(막으려는 문제보다 더 나쁜 회귀). 대신 다른 방법을 쓴다 - 시작 시각
    // 기준 경과시간 - 을 쓴다: 아무리 오래 걸리는 로컬 디버그 빌드도 RECONCILE_MAX_AGE_HOURS 를
    // 넘기면 사실상 멈췄거나 pid 재사용이 확실하므로, pid 생존 여부와 무관하게 stale 로 강제 전환해
    // "영구 running 잠금"을 막는다.
    let too_old = chrono::DateTime::parse_from_rfc3339(&job.started_at)
        .map(|started| {
            Utc::now().signed_duration_since(started) > chrono::Duration::hours(RECONCILE_MAX_AGE_HOURS)
        })
        .unwrap_or(false);
    let alive = !too_old && job.pid.map(child_env::is_pid_alive).unwrap_or(false);
    if alive {
        return Ok(job);
    }
    let mut reconciled = job;
    reconciled.status = BuildJobStatus::Failed;
    reconciled.finished_at = Some(
        reconciled
            .finished_at
            .clone()
            .unwrap_or_else(|| Utc::now().to_rfc3339()),
    );
    reconciled.exit_code = None;
    reconciled.status_note = Some(if too_old {
        format!(
            "빌드가 {RECONCILE_MAX_AGE_HOURS}시간 넘게 상태 변화가 없어서 자동으로 정리했어요 (pid가 재사용됐을 수 있어요)."
        )
    } else {
        "빌드 프로세스 상태를 더 이상 확인할 수 없어요 (비정상 종료된 것으로 보여요).".to_string()
    });

    save_single_job(base_dir, reconciled.clone())?;
    Ok(reconciled)
}

/// 로그 파일의 마지막 max_lines 줄 - 파일 없음/읽기 실패는 빈 배열로 반환한다.
fn read_log_tail(
    base_dir: &Path,
    project_id: &str,
    target: BuildTarget,
    max_lines: usize,
) -> Result<Vec<String>, String> {
    let path = log_file_path(base_dir, project_id, target);
    let mut file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return Ok(Vec::new()),
    };
    let size = file
        .metadata()
        .map_err(|e| format!("로그 파일 정보를 확인하지 못했어요: {e}"))?
        .len();
    if size == 0 {
        return Ok(Vec::new());
    }
    let start = size.saturating_sub(LOG_TAIL_MAX_BYTES);
    file.seek(SeekFrom::Start(start))
        .map_err(|e| format!("로그 파일을 읽지 못했어요: {e}"))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| format!("로그 파일을 읽지 못했어요: {e}"))?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
    // 파일이 개행으로 끝나면 마지막에 빈 문자열이 하나 더 생기니 제거.
    if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    let len = lines.len();
    Ok(lines.split_off(len.saturating_sub(max_lines)))
}

/// project 하나의 현재/마지막 빌드 상태 - job + 로그 tail + 산출물 확인 결과를 한 번에 묶어 반환한다
/// (프론트가 상태 조회 한 번으로 카드를 그릴 수 있게 하나로 묶은 모양).
pub fn get_build_status(base_dir: &Path, project: &ProjectRecord) -> Result<BuildStatus, String> {
    let jobs = load_build_jobs(base_dir)?;
    let existing = jobs.get(&project.id).cloned();
    let job = match existing {
        Some(j) => Some(reconcile_stale_job(base_dir, j)?),
        None => None,
    };
    let log_tail = match &job {
        Some(j) => read_log_tail(base_dir, &project.id, j.target, 50)?,
        None => Vec::new(),
    };
    let (artifact_relpath, artifact_exists) = match &job {
        Some(j) => {
            let rel = expected_artifact_relpath(j.target);
            let full = Path::new(&project.repo_path).join(rel);
            (Some(rel.to_string()), Some(full.exists()))
        }
        None => (None, None),
    };
    Ok(BuildStatus {
        job,
        log_tail,
        artifact_relpath,
        artifact_exists,
    })
}

/// 로그 파일에서 offset(바이트) 이후로 새로 쓰인 내용만 읽어 (텍스트, 새 offset)로 돌려준다(CLI 전용,
/// additive) - get_build_status 의 50줄 tail(read_log_tail)은 화면 카드 한 번 그리기엔 충분하지만, CLI
/// `build` 는 빌드 내내 stdout 으로 로그를 그대로 흘려보내야 해서 매 폴링마다 "마지막으로 읽은 위치
/// 이후"만 다시 읽어야 한다 - 그래야 50줄을 넘는 출력도 잃지 않는다. read_log_tail 과 달리 줄 단위가
/// 아니라 바이트 오프셋 기준이라 폴링 간격 사이에 몇 줄이 찍히든 놓치지 않는다.
///
/// 파일이 아직 없으면(로그 생성 전의 아주 좁은 창) 빈 문자열 + 같은 offset을 돌려준다(에러 아님 -
/// read_log_tail 이 파일 없음을 빈 배열로 처리하는 것과 동일한 관용). offset 이 현재 파일 크기 이상이면
/// (마지막 호출 이후 새 내용이 없음) 마찬가지로 빈 문자열 + 같은 offset.
pub fn read_log_from_offset(path: &Path, offset: u64) -> Result<(String, u64), String> {
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok((String::new(), offset)),
    };
    let size = file
        .metadata()
        .map_err(|e| format!("로그 파일 정보를 확인하지 못했어요: {e}"))?
        .len();
    if size <= offset {
        return Ok((String::new(), offset));
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("로그 파일을 읽지 못했어요: {e}"))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| format!("로그 파일을 읽지 못했어요: {e}"))?;
    let new_offset = offset + buf.len() as u64;
    Ok((String::from_utf8_lossy(&buf).into_owned(), new_offset))
}

/// read_log_from_offset 이 돌려준 청크를 줄 목록으로 쪼갠다 - read_log_tail 과 동일한 규칙으로, 청크가
/// 개행으로 끝나면 생기는 마지막 빈 문자열만 제거한다(그 외 빈 줄은 로그의 실제 빈 줄이라 보존).
/// ⚠️ 알려진 한계: 폴링 시점에 마침 한 줄이 절반만 쓰여 있으면 그 줄이 이번 청크의 마지막 "줄"로 한 번
/// 나오고, 다음 폴링에서 나머지가 또 한 번(별도 줄처럼) 나올 수 있다 - job 상태/종료 코드/산출물 판정
/// (spawn_build_job, 절대 불변)에는 전혀 영향이 없는, CLI 화면 출력만의 사소한 표시 한계다.
fn split_log_chunk(chunk: &str) -> Vec<String> {
    if chunk.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = chunk.split('\n').map(|s| s.to_string()).collect();
    if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    lines
}

/// CLI(bildorak-cli `build`)가 빌드 완료까지 포그라운드에서 기다리며 진행 상황을 받는 콜백(3단계,
/// additive) - spawn_build_job/start_build 는 이 트레이트의 존재를 전혀 모른다(완전히 바깥에서
/// get_build_status/read_log_from_offset 폴링 결과를 소비만 한다, 아래 watch_build_to_completion 문서
/// 참고). on_log 는 마지막 폴링 이후 새로 나타난 로그 줄만 넘겨받고, on_done 은 최종 BuildJob(성공/
/// 실패/취소 전부 포함)으로 정확히 한 번만 호출된다.
pub trait BuildObserver {
    fn on_log(&mut self, lines: &[String]);
    fn on_done(&mut self, job: &BuildJob);
}

/// project 의 진행 중 빌드가 끝날 때까지 interval 간격으로 get_build_status 를 폴링하며 observer 에게
/// 새 로그/완료를 알린다(3단계, additive) - start_build 가 이미 job(running)을 반환한 "다음"에 부르는
/// 게 정상 흐름이다(commands.rs::spawn_build_finish_notifier 의 폴링 패턴을 CLI 용으로 일반화한 것 -
/// AppHandle 알림 대신 임의의 BuildObserver 로 결과를 넘긴다는 점만 다르다). spawn_build_job 자체는
/// 절대 건드리지 않는다 - 이 함수는 이미 떠 있는 job 을 읽기만 한다.
///
/// 로그는 read_log_from_offset 로 바이트 오프셋 기준으로 이어 읽어(50줄 tail 유실 없이) job 이 더는
/// running 이 아닐 때까지 흘려보낸다. 완료 직후 마지막으로 한 번 더 읽어 종료 직전에 쓰인 몇 줄을
/// 놓치지 않는다.
fn pump_log_chunk(
    log_path: &Path,
    offset: &mut u64,
    observer: &mut dyn BuildObserver,
) -> Result<(), String> {
    let (chunk, new_offset) = read_log_from_offset(log_path, *offset)?;
    *offset = new_offset;
    let lines = split_log_chunk(&chunk);
    if !lines.is_empty() {
        observer.on_log(&lines);
    }
    Ok(())
}

pub fn watch_build_to_completion(
    base_dir: &Path,
    project: &ProjectRecord,
    target: BuildTarget,
    observer: &mut dyn BuildObserver,
    interval: Duration,
) -> Result<BuildJob, String> {
    let log_path = log_file_path(base_dir, &project.id, target);
    let mut offset: u64 = 0;

    loop {
        pump_log_chunk(&log_path, &mut offset, &mut *observer)?;

        let status = get_build_status(base_dir, project)?;
        let Some(job) = status.job else {
            return Err("빌드 상태를 확인하지 못했어요.".to_string());
        };
        if job.status != BuildJobStatus::Running {
            // 완료 판정 직후 남은 마지막 몇 줄을 놓치지 않도록 한 번 더 읽는다.
            pump_log_chunk(&log_path, &mut offset, &mut *observer)?;
            observer.on_done(&job);
            return Ok(job);
        }
        std::thread::sleep(interval);
    }
}

/// CLI 서브커맨드 문서 단일 소스(3단계, bildorak-cli) - bin/cli.rs 의 명령 구조가 이 목록과 동일한
/// name/설명을 쓴다(clap derive 매크로 속성은 리터럴만 받아 이 Vec 을 곧바로 끼워 넣을 수는 없어
/// cli.rs 쪽 문서 주석에도 같은 문구를 따로 적어 둔다). 실제 소비처는 둘 - (1) clap --help 로 사람이
/// 읽는 안내(cli.rs 문서 주석에 같은 문구 수동 복제), (2) GUI 설정 화면의 "CLI / 자동화" 섹션
/// (SettingsView.tsx) - commands.rs::get_cli_manifest 커맨드가 이 함수를 그대로 반환해 화면이 데이터로
/// 소비한다. 두 소비처 모두 이 함수 하나가 단일 소스다 - 여기 값을 바꾸면 화면 설명도 같이 바뀐다(다만
/// clap --help 원문은 cli.rs 쪽 문서 주석을 수동으로 맞춰야 한다, 위 참고). build.rs 에 두는 이유는 다른
/// 파일보다 더 맞는 자리가 없어서다 - 명령 6개 전부(apps/build/status/keys/doctor/releases)를 아우르는 목록이라
/// model.rs(데이터 모양만) 나 preflight.rs(점검 전용) 같은 특정 도메인 파일에 넣기도 애매하다.
pub fn cli_manifest() -> Vec<CliCommandDoc> {
    vec![
        CliCommandDoc {
            name: "apps".to_string(),
            args: String::new(),
            description: "등록된 앱(Flutter 프로젝트) 목록을 보여줘요.".to_string(),
            example: "bildorak-cli apps --json".to_string(),
        },
        CliCommandDoc {
            name: "build".to_string(),
            args: "<app> --target <android-release|ios-release|android-debug|ios-sim>".to_string(),
            description:
                "등록된 앱을 로컬에서 빌드해요. 완료될 때까지 기다리면서 로그를 그대로 보여줘요."
                    .to_string(),
            example: "bildorak-cli build myapp --target android-release".to_string(),
        },
        CliCommandDoc {
            name: "status".to_string(),
            args: "<app>".to_string(),
            description: "빌드 준비 점검 결과와 최근 빌드 상태를 보여줘요.".to_string(),
            example: "bildorak-cli status myapp".to_string(),
        },
        CliCommandDoc {
            name: "keys".to_string(),
            args: String::new(),
            description: "등록된 서명키 목록을 보여줘요(비밀번호 값은 어디에도 나오지 않아요)."
                .to_string(),
            example: "bildorak-cli keys --json".to_string(),
        },
        CliCommandDoc {
            name: "doctor".to_string(),
            args: String::new(),
            description: "Flutter/Xcode/CocoaPods/Android SDK 등 빌드 환경이 준비됐는지 점검해요."
                .to_string(),
            example: "bildorak-cli doctor".to_string(),
        },
        CliCommandDoc {
            name: "releases".to_string(),
            args: "<app>".to_string(),
            description: "등록된 앱의 릴리스 기록을 보여줘요(읽기 전용).".to_string(),
            example: "bildorak-cli releases myapp --json".to_string(),
        },
    ]
}

// ── 진행 중 빌드 레지스트리(메모리 전용) ─────────────────────────────────────
// 앱 종료 시 정리 대상(RUNNING_BUILDS)과, 동시 시작 요청 사이의 좁은 race 를 막는 락(STARTING_PROJECTS).
// 둘 다 이번 프로세스 실행 동안만 유효 - 재시작 후에는 reconcile_stale_job(파일 + pid 생존 확인)이
// 대신한다.

static RUNNING_BUILDS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
static STARTING_PROJECTS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn running_builds() -> &'static Mutex<HashMap<String, u32>> {
    RUNNING_BUILDS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn starting_projects() -> &'static Mutex<HashSet<String>> {
    STARTING_PROJECTS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 앱 종료 시(RunEvent::Exit/ExitRequested) 이번 세션에서 우리가 직접 스폰해 아직 안 끝난 빌드를 전부
/// process group kill 한다 - pid 재사용으로 엉뚱한 프로세스를 죽이는 일 없이, 이번 세션에 우리가 스폰한
/// 것이 확실한 대상만 정리한다(좀비 프로세스 금지 원칙).
pub fn kill_all_running_builds() {
    let mut map = running_builds().lock().unwrap_or_else(|e| e.into_inner());
    for (_project_id, pid) in map.drain() {
        child_env::kill_process_group(pid);
    }
}

/// starting_projects 락을 함수 종료 시(어느 반환 경로든) 자동 해제하는 RAII 가드 - `try { ... }
/// finally { ... }` 패턴을 Rust Drop 으로 옮긴 것.
struct StartGuard(String);

impl Drop for StartGuard {
    fn drop(&mut self) {
        starting_projects()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

// ── build_jobs.json 파일 자체에 대한 전역 락 ─────────────────────────────────
// 위 RUNNING_BUILDS/STARTING_PROJECTS 는 메모리 전용 레지스트리라 "파일" 경합은 못 막는다 - reconcile
// (상태 조회 시 stale 판정) / 완료 감지 스레드의 finalize / 새 spawn 의 최종 저장이 각자 따로
// load→수정→save 를 하면, 서로 다른 프로젝트를 동시에 갱신할 때 나중에 쓴 쪽이 먼저 쓴 갱신을 덮어써
// 유실될 수 있었다(설계 요구사항 - "reconcile 덮어쓰기 창 + watcher 동시 finalize 유실").
// 이 락 하나로 build_jobs.json 에 대한 모든 read-modify-write 를 직렬화한다.

static BUILD_JOBS_FILE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn build_jobs_file_lock() -> &'static Mutex<()> {
    BUILD_JOBS_FILE_LOCK.get_or_init(|| Mutex::new(()))
}

// ── build_history.json 파일 자체에 대한 전역 락(2단계) ───────────────────────
// build_jobs.json 과는 완전히 다른 파일이라 별도 락을 쓴다. append_build_history 는 finalize_build_job 이
// build_jobs_file_lock 을 놓은 "다음"에만 호출되므로(아래 finalize_build_job 참고), 이 두 락을 동시에
// 겹쳐 잡는 코드는 없다 - 재진입 불가 Mutex 두 개라도 서로 다른 순서로 물리는 경우가 없어 데드락 걱정이
// 없다.
static BUILD_HISTORY_FILE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn build_history_file_lock() -> &'static Mutex<()> {
    BUILD_HISTORY_FILE_LOCK.get_or_init(|| Mutex::new(()))
}

/// project_id 하나의 항목만 최신 상태로 갱신해 저장한다 - 락 아래서 파일을 다시 읽고(그 사이 다른
/// 프로젝트가 써 놓은 갱신을 잃지 않기 위해) 이 project_id 항목만 바꿔 쓴다. reconcile_stale_job /
/// spawn_build_job 의 최종 저장이 전부 이 함수를 거친다. std::sync::Mutex 는 재진입 불가이므로 이
/// 크레이트 안에서는 어떤 함수도 build_jobs_file_lock() 을 쥔 채로 save_single_job/finalize_build_job
/// 을 부르지 않는다(둘 다 스스로 짧게 잠그고 곧바로 푼다) - 그래야 데드락이 안 생긴다.
fn save_single_job(base_dir: &Path, job: BuildJob) -> Result<(), String> {
    let _guard = build_jobs_file_lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut jobs = load_build_jobs(base_dir)?;
    jobs.insert(job.project_id.clone(), job);
    save_build_jobs(base_dir, &jobs)
}

/// cancel_build 가 "취소 요청됨"을 표시해 두면, 실제 종료 상태를 쓰는 완료 감지 스레드가 그 표시를
/// 보고 사용자 취소로 기록한다(설계 요구사항) - 종료 상태를 쓰는 주체를 항상 완료 감지
/// 스레드 하나로 유지해 cancel_build 와 그 스레드가 build_jobs.json 을 동시에 써서 서로 덮어쓰는 새
/// race 를 만들지 않는다(위 BUILD_JOBS_FILE_LOCK 도입 취지와 동일한 원칙).
static CANCEL_REQUESTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn cancel_requested() -> &'static Mutex<HashSet<String>> {
    CANCEL_REQUESTED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 완료 감지 스레드가 호출하는 최종 상태 갱신 - job 을 다시 읽어 있으면 갱신 후 저장한다(이미 다른
/// 이유로 지워졌으면 조용히 무시). build_jobs_file_lock() 아래서 통째로 실행해 다른 프로젝트의 동시
/// finalize/reconcile/spawn 저장과 서로 덮어쓰지 않는다(설계 요구사항).
fn finalize_build_job(
    base_dir: &Path,
    project_id: &str,
    status: BuildJobStatus,
    exit_code: Option<i32>,
    status_note: Option<String>,
) -> Result<(), String> {
    // 이 블록 안의 락 범위/읽기/수정/저장 순서는 히스토리 기능 추가 전과 100% 동일하다 - 달라진 건
    // "이미 만들어진 jobs.get_mut(project_id) 결과를 finalized 에 클론해 블록 밖으로 들고 나간다"는
    // 점뿐이고, build_jobs.json 에 무엇을 쓸지/언제 쓸지는 전혀 바뀌지 않았다.
    let finalized = {
        let _guard = build_jobs_file_lock().lock().unwrap_or_else(|e| e.into_inner());
        let mut jobs = load_build_jobs(base_dir)?;
        let finalized = if let Some(job) = jobs.get_mut(project_id) {
            job.status = status;
            job.finished_at = Some(Utc::now().to_rfc3339());
            job.exit_code = exit_code;
            job.status_note = status_note;
            // job(=jobs 로부터의 mutable borrow)의 마지막 사용을 save_build_jobs(&jobs) 호출 "전"으로
            // 끝내기 위해 clone 을 먼저 뜬다(borrow checker: &mut job 과 &jobs 는 동시에 못 산다) - 저장
            // 순서/내용 자체는 그대로다(jobs 는 이미 위에서 mutate 된 상태).
            let snapshot = job.clone();
            save_build_jobs(base_dir, &jobs)?;
            Some(snapshot)
        } else {
            None
        };
        finalized
    };
    // 히스토리 기록(2단계, 순수 추가) - build_jobs_file_lock 을 놓은 뒤 별도 락으로 best-effort 수행한다.
    // append_build_history 자체가 내부에서 에러를 삼키므로(로그만 남김), 여기서 실패해도 위에서 이미
    // 끝난 build_jobs.json 상태 기록(finalize_build_job 의 기존 목적)에는 전혀 영향이 없다.
    if let Some(job) = finalized {
        append_build_history(base_dir, &job);
    }
    Ok(())
}

/// finalize_build_job 이 build_jobs.json 상태 기록을 마친 "다음"에 호출되는 best-effort 히스토리 기록
/// (2단계 추가) - 실패해도 표준에러에만 남기고 삼킨다(호출부의 Result 에 영향 없음). 프로젝트당
/// BUILD_HISTORY_MAX_PER_PROJECT 개를 넘으면 오래된 것부터 제거, 최신 항목이 목록 맨 앞에 온다.
fn append_build_history(base_dir: &Path, job: &BuildJob) {
    if let Err(e) = try_append_build_history(base_dir, job) {
        eprintln!("빌드 히스토리 기록을 남기지 못했어요(무시하고 계속 진행): {e}");
    }
}

fn try_append_build_history(base_dir: &Path, job: &BuildJob) -> Result<(), String> {
    let _guard = build_history_file_lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut history = load_build_history(base_dir)?;
    let list = history.entry(job.project_id.clone()).or_insert_with(Vec::new);
    list.insert(0, job.clone());
    list.truncate(BUILD_HISTORY_MAX_PER_PROJECT);
    save_build_history(base_dir, &history)
}

/// 저장된 빌드 히스토리를 최신순으로 돌려준다(2단계, 무료 오픈소스라 게이트 없음 - build.rs 는 이
/// 파일 전체의 원칙대로 라이선스를 모르는 경계를 유지한다). 완료된 빌드가 한 번도 없거나
/// 히스토리 파일이 아직 없으면 빈 벡터.
pub fn get_build_history(base_dir: &Path, project_id: &str) -> Result<Vec<BuildJob>, String> {
    let history = load_build_history(base_dir)?;
    Ok(history.get(project_id).cloned().unwrap_or_default())
}

// ── Android release 서명 자동 주입(다음 단계) ──────────────────────────────────────────────
// resolve_command() 의 고정 맵은 &'static str 만 다뤄 dynamic 한 keystore 경로/비밀번호를 못 담는다 -
// AndroidRelease 는 그래서 start_build() 안에서 별도로 소유(String) args 를 만들어 spawn_build_job 에
// 넘긴다. 연결된 서명키가 없거나 아직 비밀번호를 등록하지 않았으면(signing.rs::register_android_signing
// 을 아직 안 부름) 완전히 기존 그대로 동작한다(무회귀) - 이 경로는 "연결 + 비밀번호 등록"을 둘 다 마친
// 프로젝트에서만 탄다.

/// -P 인자 조립(start_build)과 빌드 후 검증(spawn_build_job 완료 감지 스레드) 양쪽이 함께 쓰는 값 -
/// key_password 는 인자 조립에만 쓰고 검증(signing::verify_release_signing)에는 store_password 만
/// 있으면 되지만, 두 값을 따로 쪼개 들고 다니는 것보다 하나로 묶어 이 구조체 하나만 완료 감지
/// 스레드로 옮기는 편이 더 단순하다(호출부 복잡도를 낮춘다).
struct ResolvedAndroidSigning {
    keystore_path: String,
    key_alias: String,
    store_password: String,
    key_password: String,
}

/// signing_keys.json 에서 이 프로젝트에 연결되어 있고 비밀번호까지 등록된 Android keystore 서명키를
/// 찾아 keychain 에서 실제 비밀번호를 읽어온다. 여러 개가 연결돼 있으면 첫 번째를 쓴다(1차 범위 -
/// SigningKeysSection UI 도 한 프로젝트에 여러 Android 키 연결을 막지 않으므로 이 모호성은 이미 UI
/// 레벨에 존재한다). 연결된 게 없거나 아직 비밀번호를 등록하지 않았으면 Ok(None) - 호출부가 기존
/// 서명-없는 빌드로 그대로 진행한다. keychain 조회 자체가 실패하면(항목이 지워졌다 등) Err - 이 경우는
/// "등록은 했다고 돼 있는데 실제로는 못 읽는" 상태라 조용히 무시하지 않고 빌드 시작 전에 바로 알린다.
fn resolve_android_signing(
    base_dir: &Path,
    project: &ProjectRecord,
) -> Result<Option<ResolvedAndroidSigning>, String> {
    let keys = signing::load_signing_keys(base_dir)?;
    let Some(key) = keys.into_iter().find(|k| {
        k.kind == SigningKeyKind::AndroidKeystore
            && k.linked_project_ids.contains(&project.id)
            && k.android_signing.is_some()
    }) else {
        return Ok(None);
    };
    let cfg = key.android_signing.expect("위 find 조건에서 is_some 확인함");
    let store_password = signing::read_keychain_password(&cfg.store_password_service, &cfg.keychain_account)?;
    let key_password = signing::read_keychain_password(&cfg.key_password_service, &cfg.keychain_account)?;
    // 실제 서명에 쓸 keystore 파일 - 안전 보관 볼트 사본이 있으면 그걸 우선 쓴다(자체 완결 원칙,
    // model.rs::SigningKeyRecord::vault_path 문서 참고: 원본이 옮겨지거나 지워져도 빌드가 깨지지
    // 않는다). 이 기능 이전에 등록된 레코드처럼 vault_path 가 없으면 원본(file_path)으로 물러난다(무회귀).
    let keystore_path = key.vault_path.unwrap_or(key.file_path);
    Ok(Some(ResolvedAndroidSigning {
        keystore_path,
        key_alias: cfg.key_alias,
        store_password,
        key_password,
    }))
}

// ── iOS release export 설정(app-store 서명, 설계 결정 2026-08) ──────────────────────────────
// 서명 자체(개인 키/프로비저닝 프로필)는 이번에도 범위 밖 - Xcode + keychain 인증서가 그대로 담당한다.
// 빌도락은 xcodebuild -exportArchive 가 요구하는 ExportOptions.plist 만 만든다. 실측(flutter build ipa
// --help, flutter 3.41.8): --export-options-plist 는 --export-method 와 동시에 못 쓴다(flutter_tools
// build_ios.dart 실측 - argParser 검증에서 "is not compatible with" 에러) - team_id 를 주입하려면
// --export-options-plist 하나만 쓰고 method/teamID/signingStyle 을 전부 이 plist 안에 직접 적어야 한다.

const IOS_EXPORT_OPTIONS_DIR: &str = "ios-export";

/// project.pbxproj 한 줄에서 DEVELOPMENT_TEAM 값을 뽑는다(우선순위 1) - 실측(이 머신): 어떤 프로젝트는
/// Debug/Release/Profile 등 build config 마다 같은 값이 여러 줄 반복되고(첫 번째 값을
/// 그대로 쓰면 된다), 어떤 프로젝트는 이 키 자체가 파일에 없다(project.pbxproj 가 자동 서명이라도 팀을
/// 기재 안 해 둔 경우 - 아래 team_id_from_pbxproj 호출부의 keychain 폴백이 담당). 값이 빈 문자열인 줄
/// (`DEVELOPMENT_TEAM = "";`)은 "미설정"과 같으므로 건너뛰고 다음 줄을 계속 본다.
fn parse_development_team(pbxproj: &str) -> Option<String> {
    for line in pbxproj.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("DEVELOPMENT_TEAM") else { continue };
        // 다른 키 이름의 접두 오탐 방지(key_scan.rs::find_gradle_string_field 와 동일한 방어) - 이 키
        // 바로 뒤는 항상 공백 또는 '=' 여야 한다.
        if !rest.starts_with(|c: char| c.is_whitespace() || c == '=') {
            continue;
        }
        let Some(rest) = rest.trim_start().strip_prefix('=') else { continue };
        let value = rest.split(';').next().unwrap_or("").trim().trim_matches('"').trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// `<repo_path>/ios/Runner.xcodeproj/project.pbxproj` 를 읽어 parse_development_team 을 적용한다.
/// 파일이 없거나 못 읽으면(프로젝트 구조가 다르거나 아직 DEVELOPMENT_TEAM 을 기재하지 않음) None -
/// 호출부(resolve_ios_team_id)가 keychain 폴백으로 이어간다(하드 에러 아님).
fn team_id_from_pbxproj(repo_path: &Path) -> Option<String> {
    let pbxproj_path = repo_path.join("ios").join("Runner.xcodeproj").join("project.pbxproj");
    let raw = fs::read_to_string(pbxproj_path).ok()?;
    parse_development_team(&raw)
}

/// iOS release export 에 쓸 Team ID 확정 - 1순위 project.pbxproj(프로젝트가 이미 특정 팀으로 서명
/// 설정을 마쳤다는 가장 확실한 신호), 2순위 keychain 에 실제 설치된 "Apple Distribution" 배포 인증서
/// (pbxproj 에 기재가 안 된 프로젝트용 폴백, signing.rs::find_distribution_team_id 가
/// `security find-identity` 로 조회). 둘 다 없으면 이 프로젝트를 어느 팀으로 서명해야 할지 알 방법이
/// 없으므로 추측하지 않고 명확한 에러로 멈춘다.
fn resolve_ios_team_id(repo_path: &Path) -> Result<String, String> {
    if let Some(team_id) = team_id_from_pbxproj(repo_path) {
        return Ok(team_id);
    }
    if let Some(team_id) = signing::find_distribution_team_id() {
        return Ok(team_id);
    }
    Err(
        "iOS 배포 팀(Team ID)을 찾지 못했어요. project.pbxproj 에 DEVELOPMENT_TEAM 이 없고, 키체인에서도 \
         Apple Distribution 배포 인증서를 찾지 못했어요. Xcode 에서 서명(Signing & Capabilities) 설정을 \
         확인해 주세요."
            .to_string(),
    )
}

/// xcodebuild -exportArchive 가 요구하는 ExportOptions.plist 최소 내용 - method 는 항상 app-store(스토어
/// 제출 고정값), signingStyle 은 automatic(이 머신에 등록된 프로젝트가 전부 Automatic signing, 실측
/// 배경), teamID 는 resolve_ios_team_id 확정값, uploadSymbols 는 크래시 심볼릭 처리를 위해 켜 둔다(합리적
/// 기본값). team_id 는 Apple 규격상 영숫자만 담는 고정폭 문자열이라 별도 XML escape 는 하지 않는다.
fn ios_export_options_plist_contents(team_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>method</key>
    <string>app-store</string>
    <key>teamID</key>
    <string>{team_id}</string>
    <key>signingStyle</key>
    <string>automatic</string>
    <key>uploadSymbols</key>
    <true/>
</dict>
</plist>
"#
    )
}

fn ios_export_options_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(IOS_EXPORT_OPTIONS_DIR)
}

fn ios_export_options_path(base_dir: &Path, project_id: &str) -> PathBuf {
    ios_export_options_dir(base_dir).join(format!("{project_id}-ExportOptions.plist"))
}

/// ExportOptions.plist 를 base_dir(app config dir) 하위에 새로 쓴다(있으면 덮어쓰기) - 프로젝트
/// 폴더(git 레포)는 절대 건드리지 않는다(설계 원칙 "원본 불변"). 매 릴리스 빌드 시작 시 team_id 를
/// 다시 확정해 덮어쓰므로 이전 빌드의 낡은 값이 남지 않는다. write_json_atomic 은 이름과 달리 순수
/// temp+rename 원자적 쓰기라 JSON 이 아닌 plist 텍스트에도 그대로 재사용한다(store.rs 문서 참고,
/// build_jobs.json 저장과 동일한 이유로 도중에 죽어도 반쪽 파일을 안 남긴다).
fn write_ios_export_options(base_dir: &Path, project_id: &str, team_id: &str) -> Result<PathBuf, String> {
    let dir = ios_export_options_dir(base_dir);
    fs::create_dir_all(&dir).map_err(|e| format!("iOS export 설정 폴더를 만들지 못했어요: {e}"))?;
    let path = ios_export_options_path(base_dir, project_id);
    let contents = ios_export_options_plist_contents(team_id);
    crate::store::write_json_atomic(&path, &contents)
        .map_err(|e| format!("ExportOptions.plist 를 만들지 못했어요: {e}"))?;
    Ok(path)
}

/// `build/ios/ipa` 디렉터리 안에 실제 .ipa 파일이 있는지 확인한다(확장자 대소문자 무시) -
/// expected_artifact_relpath(IosRelease) 는 디렉터리 하나만 가리켜서(파일명이 앱마다 달라 고정 불가,
/// 위 문서 참고) "디렉터리가 있다"만으로는 부족하다. flutter_tools 실측(build_ios.dart: xcodebuild
/// -exportArchive 가 비정상 종료해도 FlutterCommandResult.success() 를 그대로 반환)이 확인해 준 대로
/// exit code 만으로는 실제 export 성공 여부를 알 수 없어, spawn_build_job 완료 스레드가 이 함수로
/// 다시 확인한다(Android 서명 사후 검증과 같은 원칙).
fn ipa_dir_contains_ipa(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else { return false };
    entries.flatten().any(|entry| {
        entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("ipa"))
            .unwrap_or(false)
    })
}

/// 실제 spawn 로직 - bin/args 를 직접 받는다(테스트에서 빠른 가짜 커맨드로 검증하기 위함). 공개
/// 진입점인 start_build() 는 항상 resolve_command(target) 의 고정 맵(또는 AndroidRelease 서명 주입
/// 시엔 동적으로 조립한 args)만 여기로 넘기므로, 이 함수가 bin/args 를 매개변수로 받아도 엔진 원칙
/// (프론트가 문자열을 못 넣음)은 깨지지 않는다 - 이 함수 자체가 크레이트 내부 전용이라 호출자
/// (start_build, 테스트)가 신뢰된 값만 넘긴다. android_signing 이 Some 이면 빌드가 flutter 자체 종료
/// 코드 기준으로 성공한 "다음"에 signing::verify_release_signing 으로 한 번 더 확인하고, 실패하면
/// 성공을 실패로 덮어쓰고 산출물을 지운다(release 서명 검증의 핵심 게이트).
fn spawn_build_job(
    base_dir: &Path,
    project: &ProjectRecord,
    target: BuildTarget,
    bin: &str,
    args: &[&str],
    android_signing: Option<ResolvedAndroidSigning>,
) -> Result<BuildJob, String> {
    // 동시에 들어온 두 start 요청이 "진행 중인지 확인"과 "running 으로 기록" 사이의 좁은 틈에서
    // 둘 다 통과해 중복 spawn 되는 것을 막는다.
    {
        let mut starting = starting_projects().lock().unwrap_or_else(|e| e.into_inner());
        if !starting.insert(project.id.clone()) {
            return Err("이미 이 프로젝트의 빌드가 진행 중이에요. 완료된 뒤 다시 시도해 주세요.".to_string());
        }
    }
    let _guard = StartGuard(project.id.clone());

    // 이전 빌드에서 취소 플래그가 잔류했을 수 있다(watcher 종료와 cancel_build 사이 좁은 창).
    // 새 빌드 확정 시점에 반드시 비워, 이번 빌드가 엉뚱하게 "취소됨"으로 오표기되지 않게 한다.
    cancel_requested()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&project.id);

    if !Path::new(&project.repo_path).is_dir() {
        return Err(
            "등록된 프로젝트 폴더를 찾을 수 없어요. 폴더가 이동되었거나 삭제된 것 같아요.".to_string(),
        );
    }

    // 진행 중인지 확인(+ stale 이면 reconcile). 이 읽기는 락 없이 한다 - save_single_job/
    // finalize_build_job 이 temp+rename 으로 원자적으로 쓰므로 읽는 쪽은 항상 "쓰기 전" 아니면
    // "쓰기 후" 완전한 스냅샷만 본다(반쯤 쓰인 내용을 볼 일이 없다). reconcile_stale_job 이 실제로
    // 파일을 고쳐야 하면 스스로 짧게 잠그므로 여기서 미리 잠그지 않는다(재진입 불가 Mutex라 겹치면
    // 데드락).
    let jobs = load_build_jobs(base_dir)?;
    if let Some(existing) = jobs.get(&project.id).cloned() {
        let reconciled = reconcile_stale_job(base_dir, existing)?;
        if reconciled.status == BuildJobStatus::Running {
            return Err("이미 이 프로젝트의 빌드가 진행 중이에요. 완료된 뒤 다시 시도해 주세요.".to_string());
        }
    }

    let label = target_label(target).to_string();
    fs::create_dir_all(logs_dir_path(base_dir))
        .map_err(|e| format!("로그 폴더를 만들지 못했어요: {e}"))?;
    let log_path = log_file_path(base_dir, &project.id, target);
    // 이전 실행 로그는 비운다 - tail 은 항상 "이번 실행"만 보여준다.
    let log_file = fs::File::create(&log_path).map_err(|e| format!("로그 파일을 만들지 못했어요: {e}"))?;
    let log_file_for_stderr = log_file
        .try_clone()
        .map_err(|e| format!("로그 파일을 열지 못했어요: {e}"))?;

    let mut cmd = Command::new(bin);
    child_env::apply_allowlisted_env(&mut cmd);
    cmd.args(args);
    cmd.current_dir(&project.repo_path);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::from(log_file));
    cmd.stderr(Stdio::from(log_file_for_stderr));
    child_env::spawn_in_new_process_group(&mut cmd);

    let started_at = Utc::now().to_rfc3339();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let job = BuildJob {
                id: Uuid::new_v4().to_string(),
                project_id: project.id.clone(),
                target,
                target_label: label,
                status: BuildJobStatus::Failed,
                started_at: started_at.clone(),
                finished_at: Some(Utc::now().to_rfc3339()),
                exit_code: None,
                pid: None,
                status_note: Some(format!("빌드 프로세스를 시작하지 못했어요: {e}")),
            };
            save_single_job(base_dir, job.clone())?;
            return Ok(job);
        }
    };

    let pid = child.id();
    running_builds()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(project.id.clone(), pid);

    let job = BuildJob {
        id: Uuid::new_v4().to_string(),
        project_id: project.id.clone(),
        target,
        target_label: label,
        status: BuildJobStatus::Running,
        started_at,
        finished_at: None,
        exit_code: None,
        pid: Some(pid),
        status_note: None,
    };
    if let Err(e) = save_single_job(base_dir, job.clone()) {
        // 프로세스는 이미 떠 있는데 상태 저장에 실패했다 - 그대로 두면 build_jobs.json 에는 없지만
        // 백그라운드에서 계속 도는 "보이지 않는 빌드"가 된다(추적할 방법이 사라지므로 완료 시점도
        // 영영 기록되지 않는다). 안전하게 되돌리는 유일한 방법은 즉시 죽이고 레지스트리에서 지우는
        // 것이다(설계 요구사항 - "spawn 후 save 실패 정리").
        child_env::kill_process_group(pid);
        let _ = child.kill();
        let _ = child.wait();
        running_builds()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&project.id);
        return Err(e);
    }

    // 완료 감지는 별도 스레드에서 진행한다 - 이 커맨드는 이미 job(running) 을 반환했으므로 이 스레드는
    // 완료/타임아웃/취소 시점에 파일을 갱신하는 역할만 한다. blocking wait() 대신 try_wait() 폴링
    // 루프를 쓰는 건 BUILD_TIMEOUT 경과를 직접 재기 위함이다(preflight.rs 의 run_with_timeout 과
    // 같은 패턴, 설계 요구사항).
    let base_dir_owned = base_dir.to_path_buf();
    let project_id = project.id.clone();
    let project_repo_path = project.repo_path.clone();
    std::thread::spawn(move || {
        let start = Instant::now();
        let (mut status, mut exit_code, mut note) = loop {
            match child.try_wait() {
                Ok(Some(exit_status)) => {
                    break if exit_status.success() {
                        (BuildJobStatus::Success, exit_status.code(), None)
                    } else {
                        (BuildJobStatus::Failed, exit_status.code(), None)
                    };
                }
                Ok(None) => {
                    if start.elapsed() > BUILD_TIMEOUT {
                        // 무한 hang 방지 안전망 - 앱을 강제 종료하지 않아도 자동으로 정리된다
                        // (설계 요구사항).
                        child_env::kill_process_group(pid);
                        let _ = child.wait();
                        break (
                            BuildJobStatus::Failed,
                            None,
                            Some(format!(
                                "빌드가 제한 시간({}분)을 넘어서 자동으로 중단됐어요.",
                                BUILD_TIMEOUT.as_secs() / 60
                            )),
                        );
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(e) => {
                    break (
                        BuildJobStatus::Failed,
                        None,
                        Some(format!("빌드 프로세스 상태를 확인하지 못했어요: {e}")),
                    );
                }
            }
        };

        // Android release 서명 사후 검증(release 서명 검증의 핵심) - flutter 자체는 성공(exit 0)
        // 해도 injected-signing 이 조용히 무시되고 debug 키로 서명되는 사고가 실재하므로(실제 Play
        // 반려 실증), 등록 keystore 인증서와 실제 서명 인증서가 일치하는지 여기서 다시 확인한다. 취소된
        // 빌드는 이미 status != Success 라 이 블록에 들어오지 않는다(자연히 건너뜀, 별도 분기 불필요).
        if status == BuildJobStatus::Success {
            if let Some(resolved_signing) = &android_signing {
                let artifact_path = Path::new(&project_repo_path).join(expected_artifact_relpath(target));
                if let Err(e) = signing::verify_release_signing(
                    &artifact_path,
                    Path::new(&resolved_signing.keystore_path),
                    &resolved_signing.key_alias,
                    &resolved_signing.store_password,
                ) {
                    // 서명이 안 맞으면(=debug fallback 등) 빌드를 실패로 덮어쓰고 산출물을 폐기한다 -
                    // 스토어에 못 올리는 반쪽짜리 aab 를 "성공"으로 남겨두지 않는다(확정된 설계 결정).
                    let _ = fs::remove_file(&artifact_path);
                    status = BuildJobStatus::Failed;
                    exit_code = None;
                    note = Some(format!("서명이 안 맞아요, 스토어에 못 올려요. {e}"));
                } else {
                    note = Some("등록한 keystore로 서명 검증까지 완료했어요.".to_string());
                }
            }
        }

        // iOS release ipa 실제 산출물 검증(설계 결정, 2026-08) - 위 top-of-file 문단과 flutter_tools
        // 실측대로, xcodebuild -exportArchive 가 실패해도 flutter 자체 exit code 는 0(성공)을 낼 수
        // 있다. Android 서명 검증과 같은 원칙으로 "성공"을 반환하기 전에 실제 .ipa 파일이 있는지 여기서
        // 다시 확인한다 - 스토어에 못 올리는 빈 디렉터리를 "성공"으로 남겨두지 않는다.
        if status == BuildJobStatus::Success && target == BuildTarget::IosRelease {
            let ipa_dir = Path::new(&project_repo_path).join(expected_artifact_relpath(target));
            if !ipa_dir_contains_ipa(&ipa_dir) {
                status = BuildJobStatus::Failed;
                exit_code = None;
                note = Some(
                    "빌드는 끝났지만 .ipa 파일이 실제로 만들어지지 않았어요. Xcode 서명/팀 설정을 확인해 \
                     주세요."
                        .to_string(),
                );
            }
        }

        // cancel_build 가 남긴 "취소 요청됨" 표시가 있으면 타임아웃/실패 문구 대신 취소 문구로
        // 덮어쓴다 - 프로세스를 죽이는 kill_process_group 호출은 cancel_build 가 이미 했으므로 여기
        // 루프는 그 결과(exit_status 실패)를 그대로 감지해 반영할 뿐이다.
        let note = if cancel_requested()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&project_id)
        {
            Some("사용자가 빌드를 취소했어요.".to_string())
        } else {
            note
        };
        let _ = finalize_build_job(&base_dir_owned, &project_id, status, exit_code, note);
        // running_builds 에서 빼는 건 finalize 가 파일에 진짜 결과를 다 쓴 "다음"이어야 한다 - 먼저
        // 빼버리면 그 사이 짧은 틈에 reconcile_stale_job 이 "로컬 감지 스레드 없음"으로 오판해 아직
        // 파일에 안 쓰인 running 상태를 stale 로 덮어쓸 수 있다(위 reconcile_stale_job 주석과 짝).
        running_builds()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&project_id);
    });

    Ok(job)
}

/// 진행 중인 빌드를 취소한다 - 프론트의 "빌드 취소" 버튼이 호출한다(설계 요구사항).
/// 이번 세션이 직접 스폰해 완료 감지 스레드가 떠 있는 경우(running_builds 에 등록됨)에는 여기서는
/// 프로세스 그룹만 죽이고 "취소 요청됨" 표시만 남긴다 - 최종 상태 기록은 그 스레드(finalize_build_job)
/// 에 맡긴다. 그래야 종료 상태를 쓰는 주체가 항상 하나로 유지되어, cancel_build 와 완료 감지 스레드가
/// build_jobs.json 을 동시에 써서 서로 덮어쓰는 새 race 를 만들지 않는다.
///
/// 이번 세션이 스폰하지 않은 running 기록(예: 재시작 이전에 시작된 빌드라 완료 감지 스레드가 없음)
/// 이면 대신할 스레드가 없으므로 여기서 직접 죽이고 최종 상태까지 기록한다.
pub fn cancel_build(base_dir: &Path, project: &ProjectRecord) -> Result<(), String> {
    let local_pid = running_builds()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&project.id)
        .copied();

    if let Some(pid) = local_pid {
        cancel_requested()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(project.id.clone());
        child_env::kill_process_group(pid);
        return Ok(());
    }

    let jobs = load_build_jobs(base_dir)?;
    let job = jobs
        .get(&project.id)
        .cloned()
        .filter(|j| j.status == BuildJobStatus::Running);
    let Some(mut job) = job else {
        return Err("취소할 진행 중인 빌드를 찾지 못했어요.".to_string());
    };
    if let Some(pid) = job.pid {
        child_env::kill_process_group(pid);
    }
    job.status = BuildJobStatus::Failed;
    job.finished_at = Some(Utc::now().to_rfc3339());
    job.exit_code = None;
    job.status_note = Some("사용자가 빌드를 취소했어요.".to_string());
    save_single_job(base_dir, job)
}

/// 공개 진입점 - commands.rs 가 여기만 호출한다. bin/args 는 항상 resolve_command(target) 의 고정
/// 맵에서만 나온다(프론트는 target enum 만 고를 수 있다) - release 두 타겟만 예외다: AndroidRelease 는
/// 연결된 서명키에 비밀번호가 등록돼 있으면 flutter 고정 인자 뒤에 -P 서명 property 4개를 추가로
/// 붙이고, IosRelease 는 항상 --export-options-plist 하나를 추가로 붙인다(위 "iOS release export 설정"
/// 절 참고). 그래도 bin/base args 는 여전히 고정 맵 또는 이 함수 안에서만 조립되고, 프론트는 여전히
/// target enum 만 고를 뿐 문자열을 못 넣는다(엔진 원칙 유지). 무료 오픈소스라 release 타겟도 게이트
/// 없이 그대로 쓸 수 있다(commands.rs::start_build 문서 참고 - 이 함수는 이 파일 전체 원칙대로
/// 라이선스를 모르는 경계를 유지 - get_build_history/append_build_history 와 동일).
pub fn start_build(
    base_dir: &Path,
    project: &ProjectRecord,
    target: BuildTarget,
) -> Result<BuildJob, String> {
    // iOS 는 시뮬레이터/디바이스 어느 쪽이든 Xcode 가 있어야 빌드된다 - macOS 전용 제약은 IosSimDebug
    // 뿐 아니라 IosRelease(flutter build ipa)에도 그대로 적용된다.
    if matches!(target, BuildTarget::IosSimDebug | BuildTarget::IosRelease) && !cfg!(target_os = "macos") {
        return Err("iOS 빌드는 macOS 에서만 실행할 수 있어요.".to_string());
    }

    // 설정 화면(settings.rs)에 등록된 Flutter SDK 경로가 있으면 그 경로를 쓰고, 없으면 기존 그대로
    // PATH 의 "flutter"를 쓴다(settings::resolve_flutter_bin, 미설정 시 완전히 동일 동작 - 무회귀,
    // settings.rs 의 tests 모듈이 이 폴백을 실측 검증한다). 아래 세 경로(release 둘 + resolve_command
    // 폴백) 전부 이 한 값을 쓴다 - 각 argv(고정 인자 목록) 자체는 전혀 바뀌지 않는다.
    let flutter_bin = crate::settings::resolve_flutter_bin(base_dir);

    if target == BuildTarget::IosRelease {
        // app-store export 설정은 항상 만든다(Android 처럼 "연결된 키가 없으면 건너뛴다"는 선택지가
        // 없다 - ExportOptions.plist 없이는 team_id 를 주입할 방법 자체가 없다, 위 top-of-file 문단
        // 참고). 팀을 못 찾으면(project.pbxproj 도, keychain 도) 여기서 바로 Err - spawn 자체를 하지
        // 않는다(Android 의 resolve_android_signing(..)? 과 동일한 "실패는 spawn 전에" 원칙).
        let team_id = resolve_ios_team_id(Path::new(&project.repo_path))?;
        let export_options_path = write_ios_export_options(base_dir, &project.id, &team_id)?;
        let owned_args: Vec<String> = vec![
            "build".to_string(),
            "ipa".to_string(),
            "--release".to_string(),
            format!("--export-options-plist={}", export_options_path.to_string_lossy()),
        ];
        let args_refs: Vec<&str> = owned_args.iter().map(String::as_str).collect();
        return spawn_build_job(base_dir, project, target, &flutter_bin, &args_refs, None);
    }

    if target == BuildTarget::AndroidRelease {
        if let Some(resolved_signing) = resolve_android_signing(base_dir, project)? {
            // -P 는 반드시 별도 토큰으로(고정 argv, 셸 조립 금지) - flutter build appbundle --help 실측:
            // "-P, --android-project-arg ... key=value" 형태로 반복 지정 가능.
            // ⚠️ 보안 주의: 이 args 에 `--verbose` / gradle `--info` / `--debug` 를 절대
            // 추가하지 마라. 그러면 gradle 이 `-P ...password=` 값을 stdout 으로 echo 하고, 그게 빌드 로그
            // 파일(build-logs)에 보존돼 keystore 비밀번호가 평문으로 샌다. 기본 verbosity 라 현재는 안전.
            let owned_args: Vec<String> = vec![
                "build".to_string(),
                "appbundle".to_string(),
                "--release".to_string(),
                "-P".to_string(),
                format!("android.injected.signing.store.file={}", resolved_signing.keystore_path),
                "-P".to_string(),
                format!("android.injected.signing.store.password={}", resolved_signing.store_password),
                "-P".to_string(),
                format!("android.injected.signing.key.alias={}", resolved_signing.key_alias),
                "-P".to_string(),
                format!("android.injected.signing.key.password={}", resolved_signing.key_password),
            ];
            let args_refs: Vec<&str> = owned_args.iter().map(String::as_str).collect();
            return spawn_build_job(base_dir, project, target, &flutter_bin, &args_refs, Some(resolved_signing));
        }
    }

    let (_, args) = resolve_command(target);
    spawn_build_job(base_dir, project, target, &flutter_bin, &args, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Platform;

    fn temp_base_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bildorak-test-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("failed to create temp base dir");
        dir
    }

    fn fake_project(repo_path: &Path) -> ProjectRecord {
        ProjectRecord {
            id: format!("test-{}", Uuid::new_v4()),
            name: "테스트 프로젝트".to_string(),
            selected_path: repo_path.to_string_lossy().to_string(),
            repo_path: repo_path.to_string_lossy().to_string(),
            version: None,
            build_number: None,
            platforms: vec![Platform::Ios],
            registered_at: Utc::now().to_rfc3339(),
        }
    }

    /// 백그라운드 스레드가 job 을 Running 이 아닌 상태로 갱신할 때까지 짧게 폴링한다(프론트의 실제
    /// 폴링과 같은 방식 - busy-wait 아닌 100ms 간격).
    fn wait_for_finish(base_dir: &Path, project: &ProjectRecord, timeout: Duration) -> BuildJob {
        let start = Instant::now();
        loop {
            let status = get_build_status(base_dir, project).expect("status should load");
            if let Some(job) = &status.job {
                if job.status != BuildJobStatus::Running {
                    return job.clone();
                }
            }
            if start.elapsed() > timeout {
                panic!("빌드 job 이 timeout 안에 끝나지 않았어요");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    fn resolve_command_matches_fixed_map() {
        assert_eq!(
            resolve_command(BuildTarget::IosSimDebug),
            ("flutter", vec!["build", "ios", "--simulator", "--debug"])
        );
        assert_eq!(
            resolve_command(BuildTarget::AndroidDebug),
            ("flutter", vec!["build", "apk", "--debug"])
        );
        assert_eq!(
            resolve_command(BuildTarget::IosRelease),
            ("flutter", vec!["build", "ipa", "--release"])
        );
        assert_eq!(
            resolve_command(BuildTarget::AndroidRelease),
            ("flutter", vec!["build", "appbundle", "--release"])
        );
    }

    #[test]
    fn expected_artifact_relpath_matches_target() {
        assert_eq!(
            expected_artifact_relpath(BuildTarget::IosSimDebug),
            "build/ios/iphonesimulator/Runner.app"
        );
        assert_eq!(
            expected_artifact_relpath(BuildTarget::AndroidDebug),
            "build/app/outputs/flutter-apk/app-debug.apk"
        );
        assert_eq!(expected_artifact_relpath(BuildTarget::IosRelease), "build/ios/ipa");
        assert_eq!(
            expected_artifact_relpath(BuildTarget::AndroidRelease),
            "build/app/outputs/bundle/release/app-release.aab"
        );
    }

    // ── Android release 서명 자동 주입(다음 단계) - resolve_android_signing 무회귀 + 연결 경로 ──────

    #[test]
    fn resolve_android_signing_returns_none_without_linked_key() {
        // signing_keys.json 자체가 없는(첫 실행) 프로젝트 - AndroidRelease 빌드가 기존 그대로(서명
        // 주입 없이) 진행되어야 한다는 무회귀 요구사항의 핵심 분기.
        let base_dir = temp_base_dir("android-signing-none");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        let resolved = resolve_android_signing(&base_dir, &project).expect("조회 자체는 실패하면 안 된다");
        assert!(resolved.is_none(), "연결된 서명키가 없으면 None 이어야 한다");

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    #[test]
    fn resolve_android_signing_reads_linked_key_secrets_from_keychain() {
        // signing.rs::register_android_signing 로 실제 keychain 에 저장한 뒤, build.rs 쪽에서
        // 프로젝트에 연결된 서명키를 찾아 keychain 에서 두 비밀번호를 제대로 읽어오는지 확인한다
        // (start_build 의 -P 인자 조립이 실제로 쓰는 값과 같은 경로).
        let base_dir = temp_base_dir("android-signing-linked");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        let key_id = Uuid::new_v4().to_string();
        // 실존하지 않는 더미 경로 - 이 테스트는 keychain 저장/조회 경로만 검증한다(cert 메타데이터
        // 추출은 best-effort 라 실패해도 조용히 None, signing.rs 쪽 e2e 테스트가 실제 keystore 로 따로
        // 검증한다). 아래 record.file_path 와 같은 값을 써서 시나리오를 일관되게 유지한다.
        let config =
            crate::signing::register_android_signing(Path::new("/tmp/fake-release.jks"), &key_id, "release-alias", "storepw", "keypw")
                .expect("keychain 저장 실패하면 안 된다");
        let record = crate::model::SigningKeyRecord {
            id: key_id.clone(),
            kind: SigningKeyKind::AndroidKeystore,
            display_name: "release.jks".to_string(),
            file_path: "/tmp/fake-release.jks".to_string(),
            expires_at: None,
            linked_project_ids: vec![project.id.clone()],
            android_signing: Some(config.clone()),
            // vault_path 없음 - 이 기능 이전 레코드 시나리오. keystore_path 는 file_path(원본)로 그대로
            // 물러나야 한다(아래 assert, 무회귀 확인).
            vault_path: None,
        };
        crate::signing::save_signing_keys(&base_dir, &[record]).expect("저장 실패하면 안 된다");

        let resolved = resolve_android_signing(&base_dir, &project)
            .expect("조회 실패하면 안 된다")
            .expect("연결 + 등록된 키가 있으면 Some 이어야 한다");
        assert_eq!(resolved.keystore_path, "/tmp/fake-release.jks");
        assert_eq!(resolved.key_alias, "release-alias");
        assert_eq!(resolved.store_password, "storepw");
        assert_eq!(resolved.key_password, "keypw");

        crate::signing::forget_android_signing_secrets(&config);
        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    #[test]
    fn resolve_android_signing_prefers_vault_path_when_present() {
        // keystore 안전 보관(볼트 복사) - vault_path 가 채워져 있으면 실제 서명은 원본(file_path) 대신
        // 볼트 사본을 써야 한다(자체 완결 원칙, model.rs::SigningKeyRecord::vault_path 문서 참고).
        let base_dir = temp_base_dir("android-signing-vault-preferred");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        let key_id = Uuid::new_v4().to_string();
        let config =
            crate::signing::register_android_signing(Path::new("/tmp/fake-release.jks"), &key_id, "release-alias", "storepw", "keypw")
                .expect("keychain 저장 실패하면 안 된다");
        let record = crate::model::SigningKeyRecord {
            id: key_id.clone(),
            kind: SigningKeyKind::AndroidKeystore,
            display_name: "release.jks".to_string(),
            file_path: "/tmp/fake-release.jks".to_string(),
            expires_at: None,
            linked_project_ids: vec![project.id.clone()],
            android_signing: Some(config.clone()),
            vault_path: Some("/tmp/bildorak-vault/fake-key-id-release.jks".to_string()),
        };
        crate::signing::save_signing_keys(&base_dir, &[record]).expect("저장 실패하면 안 된다");

        let resolved = resolve_android_signing(&base_dir, &project)
            .expect("조회 실패하면 안 된다")
            .expect("연결 + 등록된 키가 있으면 Some 이어야 한다");
        assert_eq!(
            resolved.keystore_path, "/tmp/bildorak-vault/fake-key-id-release.jks",
            "vault_path 가 있으면 원본(file_path) 대신 볼트 사본 경로를 써야 한다"
        );

        crate::signing::forget_android_signing_secrets(&config);
        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    #[test]
    fn fast_fake_command_failure_marks_job_failed() {
        let base_dir = temp_base_dir("fail");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        // "false" 는 즉시 exit code 1 로 끝나는 표준 coreutil - 실패 경로를 몇 초 안에 검증한다.
        let job = spawn_build_job(&base_dir, &project, BuildTarget::IosSimDebug, "false", &[], None)
            .expect("spawn should succeed");
        assert_eq!(job.status, BuildJobStatus::Running);

        let final_job = wait_for_finish(&base_dir, &project, Duration::from_secs(10));
        assert_eq!(final_job.status, BuildJobStatus::Failed);
        assert_eq!(final_job.exit_code, Some(1));

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    #[test]
    fn fast_fake_command_success_marks_job_success() {
        let base_dir = temp_base_dir("success");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        let job = spawn_build_job(&base_dir, &project, BuildTarget::AndroidDebug, "true", &[], None)
            .expect("spawn should succeed");
        assert_eq!(job.status, BuildJobStatus::Running);

        let final_job = wait_for_finish(&base_dir, &project, Duration::from_secs(10));
        assert_eq!(final_job.status, BuildJobStatus::Success);
        assert_eq!(final_job.exit_code, Some(0));

        // 로그 파일은 생겼지만(빈 파일) "true" 는 진짜 산출물을 안 만든다 - get_build_status 로 확인.
        let status = get_build_status(&base_dir, &project).expect("status should load");
        assert_eq!(status.artifact_exists, Some(false));

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    #[test]
    fn duplicate_start_while_running_is_rejected() {
        let base_dir = temp_base_dir("dup");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        // sleep 2초짜리를 띄워 "진행 중" 상태를 유지한 뒤, 같은 프로젝트로 재요청이 거부되는지 본다
        // (여기서 검증하는 건 파일에 저장된 running + pid 생존 확인 경로 - reconcile_stale_job 이
        // "아직 살아있다"고 판단해 두 번째 spawn_build_job 을 거부하는지).
        let _job = spawn_build_job(&base_dir, &project, BuildTarget::IosSimDebug, "sleep", &["2"], None)
            .expect("spawn should succeed");
        let second = spawn_build_job(&base_dir, &project, BuildTarget::IosSimDebug, "true", &[], None);
        assert!(second.is_err());

        let _ = wait_for_finish(&base_dir, &project, Duration::from_secs(10));
        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    /// 설계 요구사항 - build_jobs.json 이 손상돼 있어도(JSON 파싱 실패) 하드 에러로 빌드
    /// 기능 전체가 막히지 않고, 손상 파일을 백업한 뒤 빈 목록으로 self-heal 해야 한다.
    #[test]
    fn corrupt_build_jobs_file_self_heals() {
        let base_dir = temp_base_dir("corrupt");
        fs::write(jobs_file_path(&base_dir), "{ 이건 유효한 JSON 이 아니에요").unwrap();

        let jobs = load_build_jobs(&base_dir).expect("파싱 실패가 하드 에러면 안 된다");
        assert!(jobs.is_empty());

        let backups: Vec<_> = fs::read_dir(&base_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1, "손상 파일이 .corrupt-* 로 백업돼야 한다");

        let _ = fs::remove_dir_all(&base_dir);
    }

    /// 2단계(빌드 히스토리) - 빌드가 실제로 끝나면(완료 감지 스레드가 finalize_build_job 을 부르면) 그
    /// job 스냅샷이 build_history.json 에도 남아야 한다. spawn_build_job → 완료 감지 스레드 →
    /// finalize_build_job 전체 파이프라인을 그대로 태워 히스토리 wiring 자체를 검증한다(build_jobs.json
    /// 쪽 검증은 기존 fast_fake_command_success_marks_job_success 와 동일 - 여기서는 히스토리만 본다).
    #[test]
    fn build_history_records_completed_job() {
        let base_dir = temp_base_dir("history-basic");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        let job = spawn_build_job(&base_dir, &project, BuildTarget::AndroidDebug, "true", &[], None)
            .expect("spawn should succeed");

        let final_job = wait_for_finish(&base_dir, &project, Duration::from_secs(10));
        assert_eq!(final_job.status, BuildJobStatus::Success);

        let history = get_build_history(&base_dir, &project.id).expect("history should load");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, job.id);
        assert_eq!(history[0].status, BuildJobStatus::Success);

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    /// 2단계 - 히스토리는 프로젝트당 BUILD_HISTORY_MAX_PER_PROJECT 개까지만 보관하고(초과분은 오래된
    /// 것부터 제거), 최신 항목이 맨 앞에 온다. 실제 프로세스를 매번 띄우지 않고 save_single_job +
    /// finalize_build_job 을 직접 반복 호출해(각 반복이 "새 빌드 시작 → 완료"를 흉내낸다) 빠르게
    /// 검증한다.
    #[test]
    fn build_history_caps_at_max_and_orders_newest_first() {
        let base_dir = temp_base_dir("history-cap");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        let total = BUILD_HISTORY_MAX_PER_PROJECT + 2;
        for i in 0..total {
            let job = BuildJob {
                id: format!("job-{i}"),
                project_id: project.id.clone(),
                target: BuildTarget::IosSimDebug,
                target_label: target_label(BuildTarget::IosSimDebug).to_string(),
                status: BuildJobStatus::Running,
                started_at: Utc::now().to_rfc3339(),
                finished_at: None,
                exit_code: None,
                pid: None,
                status_note: None,
            };
            save_single_job(&base_dir, job).expect("save should succeed");
            finalize_build_job(&base_dir, &project.id, BuildJobStatus::Success, Some(0), None)
                .expect("finalize should succeed");
        }

        let history = get_build_history(&base_dir, &project.id).expect("history should load");
        assert_eq!(history.len(), BUILD_HISTORY_MAX_PER_PROJECT);
        // 최신(마지막으로 finalize 된 job-<total-1>)이 맨 앞이어야 한다.
        assert_eq!(history[0].id, format!("job-{}", total - 1));
        // 가장 오래된 2개(job-0, job-1)는 잘려나가고 없어야 한다.
        assert!(!history.iter().any(|j| j.id == "job-0"));
        assert!(!history.iter().any(|j| j.id == "job-1"));

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    /// 히스토리 파일도 build_jobs.json 과 같은 self-heal 원칙을 따른다 - 손상돼 있어도 하드 에러 대신
    /// 백업 후 빈 목록으로 시작한다.
    #[test]
    fn corrupt_build_history_file_self_heals() {
        let base_dir = temp_base_dir("history-corrupt");
        fs::write(history_file_path(&base_dir), "{ 이것도 유효한 JSON 이 아니에요").unwrap();

        let history = load_build_history(&base_dir).expect("파싱 실패가 하드 에러면 안 된다");
        assert!(history.is_empty());

        let backups: Vec<_> = fs::read_dir(&base_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1, "손상된 히스토리 파일이 .corrupt-* 로 백업돼야 한다");

        let _ = fs::remove_dir_all(&base_dir);
    }

    /// 설계 요구사항 - pid 는 실제로 살아있어도(다른 프로세스가 그 번호를 재사용) 시작
    /// 시각이 RECONCILE_MAX_AGE_HOURS 를 넘었으면 reconcile 이 stale 로 강제 전환해야 한다("영구
    /// running 잠금" 방지). running_builds 에 등록하지 않은 채(=이번 세션이 스폰하지 않은 것처럼)
    /// 진짜 프로세스를 하나 띄워, pid 생존만으로는 stale 판정을 피할 수 없음을 확인한다.
    #[test]
    fn reconcile_marks_stale_by_age_even_if_pid_alive() {
        let base_dir = temp_base_dir("age-stale");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        let mut bg = Command::new("sleep")
            .arg("5")
            .spawn()
            .expect("failed to spawn background process");
        let pid = bg.id();

        let old_job = BuildJob {
            id: Uuid::new_v4().to_string(),
            project_id: project.id.clone(),
            target: BuildTarget::IosSimDebug,
            target_label: target_label(BuildTarget::IosSimDebug).to_string(),
            status: BuildJobStatus::Running,
            started_at: (Utc::now() - chrono::Duration::hours(RECONCILE_MAX_AGE_HOURS + 1)).to_rfc3339(),
            finished_at: None,
            exit_code: None,
            pid: Some(pid),
            status_note: None,
        };

        assert!(
            child_env::is_pid_alive(pid),
            "테스트 전제: 백그라운드 프로세스가 살아있어야 한다"
        );

        let reconciled = reconcile_stale_job(&base_dir, old_job).expect("reconcile should succeed");
        assert_eq!(reconciled.status, BuildJobStatus::Failed);

        let _ = bg.kill();
        let _ = bg.wait();
        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    /// 설계 요구사항 - 진행 중인 빌드를 cancel_build 로 취소하면 process group 이
    /// 죽고, 완료 감지 스레드가 그 종료를 감지해 "사용자가 취소했어요" 문구로 최종 상태를 남긴다.
    #[test]
    fn cancel_build_kills_and_marks_failed_with_cancel_note() {
        let base_dir = temp_base_dir("cancel");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        let job = spawn_build_job(&base_dir, &project, BuildTarget::IosSimDebug, "sleep", &["30"], None)
            .expect("spawn should succeed");
        assert_eq!(job.status, BuildJobStatus::Running);

        cancel_build(&base_dir, &project).expect("cancel should succeed");

        let final_job = wait_for_finish(&base_dir, &project, Duration::from_secs(10));
        assert_eq!(final_job.status, BuildJobStatus::Failed);
        assert_eq!(
            final_job.status_note.as_deref(),
            Some("사용자가 빌드를 취소했어요.")
        );

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    /// cancel_build 는 이번 세션이 스폰하지 않은(=running_builds 에 없는) running 기록도 처리할 수
    /// 있어야 한다(재시작 이후 완료 감지 스레드가 없는 경우) - pid 를 직접 죽이고 최종 상태까지
    /// 스스로 기록하는 경로를 검증한다.
    #[test]
    fn cancel_build_without_local_watcher_kills_and_writes_state() {
        let base_dir = temp_base_dir("cancel-no-watcher");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        // kill_process_group 은 process group 전체(`kill -9 -<pid>`)를 겨냥하므로, 진짜 빌드처럼
        // 새 process group 의 리더로 띄워야 한다 - 그냥 spawn() 하면 이 테스트 실행기(cargo test)의
        // 그룹을 물려받아 `-<pid>` 가 가리키는 그룹이 애초에 존재하지 않아 kill 이 조용히 실패한다.
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        child_env::spawn_in_new_process_group(&mut cmd);
        let mut bg = cmd.spawn().expect("failed to spawn background process");
        let pid = bg.id();

        let job = BuildJob {
            id: Uuid::new_v4().to_string(),
            project_id: project.id.clone(),
            target: BuildTarget::IosSimDebug,
            target_label: target_label(BuildTarget::IosSimDebug).to_string(),
            status: BuildJobStatus::Running,
            started_at: Utc::now().to_rfc3339(),
            finished_at: None,
            exit_code: None,
            pid: Some(pid),
            status_note: None,
        };
        save_single_job(&base_dir, job).expect("save should succeed");

        cancel_build(&base_dir, &project).expect("cancel should succeed");

        // cancel_build 는 pid 만 알 뿐 Child 핸들이 없어 reap 하지 못한다(실제 크로스-재시작 상황에서
        // 죽이는 대상은 이전 앱 인스턴스의 고아 프로세스라 launchd/init 이 대신 reap 한다). 이 테스트는
        // bg 의 진짜 OS 부모라 우리가 직접 reap 해야 좀비 상태(kill -0 이 여전히 "존재"로 보는 상태)를
        // 지나 진짜로 사라진 뒤에 생존 여부를 확인할 수 있다.
        let _ = bg.wait();

        let status = get_build_status(&base_dir, &project).expect("status should load");
        let final_job = status.job.expect("job should exist");
        assert_eq!(final_job.status, BuildJobStatus::Failed);
        assert_eq!(
            final_job.status_note.as_deref(),
            Some("사용자가 빌드를 취소했어요.")
        );
        assert!(!child_env::is_pid_alive(pid), "취소되면 실제 프로세스도 죽어야 한다");

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    /// e2e: 실제 프로젝트로 iOS 시뮬레이터 디버그 빌드를 job 파이프라인 통째로 실행한다(수동 회귀
    /// 검증용). 수 분 걸릴 수 있어 평소 `cargo test` 에서는 --ignored 로 제외한다:
    ///   cargo test --manifest-path src-tauri/Cargo.toml e2e_ios_sim_build_real_project -- --ignored --nocapture
    #[test]
    #[ignore]
    fn e2e_ios_sim_build_real_project() {
        // 로컬에서 이 테스트를 실제로 돌리려면 아래 경로를 실제 Flutter 프로젝트 위치로 바꿔야 한다.
        let repo_path = PathBuf::from("/Users/you/projects/myapp/app");
        assert!(
            repo_path.join("pubspec.yaml").is_file(),
            "pubspec.yaml 를 찾지 못했어요 - repo_path 를 실제 로컬 Flutter 프로젝트 경로로 바꿔서 실행하세요"
        );

        let base_dir = temp_base_dir("e2e-real-project");
        let project = fake_project(&repo_path);

        let (bin, args) = resolve_command(BuildTarget::IosSimDebug);
        let job = spawn_build_job(&base_dir, &project, BuildTarget::IosSimDebug, bin, &args, None)
            .expect("실제 빌드 spawn 이 성공해야 해요");
        assert_eq!(job.status, BuildJobStatus::Running);
        assert!(job.pid.is_some());

        let final_job = wait_for_finish(&base_dir, &project, Duration::from_secs(480));
        assert_eq!(
            final_job.status,
            BuildJobStatus::Success,
            "실제 flutter build 가 실패했어요: {:?}",
            final_job.status_note
        );

        let status = get_build_status(&base_dir, &project).expect("status should load");
        assert_eq!(
            status.artifact_exists,
            Some(true),
            "Runner.app 산출물이 실제로 생성되지 않았어요"
        );
        assert!(!status.log_tail.is_empty(), "로그 tail 이 비어 있어요 - 로그 파일 연결을 확인하세요");

        let _ = fs::remove_dir_all(&base_dir);
        // 실제 빌드 산출물(<repo_path>/build/ios/iphonesimulator/Runner.app)은 그 프로젝트 저장소
        // 안에 생기므로 테스트가 지우지 않는다 - 필요하면 직접 정리.
    }

    // ── iOS release export 설정(설계 결정 2026-08) ──────────────────────────────────────────

    #[test]
    fn parse_development_team_finds_value_repeated_across_build_configs() {
        // 실측 형태 - Debug/Release/Profile 마다 같은 값이 반복된다.
        let raw = "\t\t\t\tDEVELOPMENT_TEAM = ABCDE12345;\n\t\t\t\tDEVELOPMENT_TEAM = ABCDE12345;\n";
        assert_eq!(parse_development_team(raw), Some("ABCDE12345".to_string()));
    }

    #[test]
    fn parse_development_team_skips_empty_value_and_uses_next_nonempty() {
        let raw = "\t\t\t\tDEVELOPMENT_TEAM = \"\";\n\t\t\t\tDEVELOPMENT_TEAM = ABCD123456;\n";
        assert_eq!(parse_development_team(raw), Some("ABCD123456".to_string()));
    }

    #[test]
    fn parse_development_team_none_when_key_absent() {
        // 실측 - 이 키 자체가 파일에 없다.
        let raw = "\t\t\t\tPRODUCT_BUNDLE_IDENTIFIER = com.example.myapp;\n";
        assert_eq!(parse_development_team(raw), None);
    }

    #[test]
    fn parse_development_team_ignores_prefix_false_positive() {
        // "DEVELOPMENT_TEAM" 로 시작하지만 다른 키인 경우 오탐하면 안 된다(key_scan.rs 의
        // applicationIdSuffix 방어와 동일 취지).
        let raw = "\t\t\t\tDEVELOPMENT_TEAMS_SOMETHING = XXXXXXXXXX;\n\t\t\t\tDEVELOPMENT_TEAM = REAL123456;\n";
        assert_eq!(parse_development_team(raw), Some("REAL123456".to_string()));
    }

    #[test]
    fn team_id_from_pbxproj_reads_real_project_layout() {
        let repo_dir = std::env::temp_dir().join(format!("bildorak-pbxproj-repo-{}", Uuid::new_v4()));
        let pbxproj_path = repo_dir.join("ios/Runner.xcodeproj/project.pbxproj");
        fs::create_dir_all(pbxproj_path.parent().unwrap()).unwrap();
        fs::write(&pbxproj_path, "\t\t\t\tDEVELOPMENT_TEAM = ABCDE12345;\n").unwrap();
        assert_eq!(team_id_from_pbxproj(&repo_dir), Some("ABCDE12345".to_string()));
        let _ = fs::remove_dir_all(&repo_dir);
    }

    #[test]
    fn team_id_from_pbxproj_none_when_file_missing() {
        let repo_dir = std::env::temp_dir().join(format!("bildorak-pbxproj-missing-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        assert_eq!(team_id_from_pbxproj(&repo_dir), None);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    #[test]
    fn resolve_ios_team_id_uses_pbxproj_without_touching_keychain() {
        // pbxproj 에 값이 있으면 keychain(signing::find_distribution_team_id) 폴백까지 갈 필요가 없다 -
        // 이 테스트는 그 히트 경로만 검증한다(키체인 상태는 머신마다 달라 여기서 단정하지 않는다).
        let repo_dir = std::env::temp_dir().join(format!("bildorak-resolve-team-repo-{}", Uuid::new_v4()));
        let pbxproj_path = repo_dir.join("ios/Runner.xcodeproj/project.pbxproj");
        fs::create_dir_all(pbxproj_path.parent().unwrap()).unwrap();
        fs::write(&pbxproj_path, "\t\t\t\tDEVELOPMENT_TEAM = ABCDE12345;\n").unwrap();
        assert_eq!(resolve_ios_team_id(&repo_dir), Ok("ABCDE12345".to_string()));
        let _ = fs::remove_dir_all(&repo_dir);
    }

    #[test]
    fn write_ios_export_options_contains_team_id_and_app_store_defaults() {
        let base_dir = temp_base_dir("export-options-write");
        let path = write_ios_export_options(&base_dir, "project-1", "ABCDE12345")
            .expect("쓰기 실패하면 안 된다");
        let contents = fs::read_to_string(&path).expect("쓴 파일을 다시 읽지 못했다");
        assert!(contents.contains("<string>app-store</string>"));
        assert!(contents.contains("<string>ABCDE12345</string>"));
        assert!(contents.contains("<string>automatic</string>"));
        assert!(path.starts_with(&base_dir), "프로젝트 폴더가 아니라 base_dir 밑에 써야 한다");
        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn ipa_dir_contains_ipa_true_when_ipa_file_present() {
        let dir = temp_base_dir("ipa-dir-present");
        fs::write(dir.join("Runner.ipa"), b"dummy ipa bytes").unwrap();
        assert!(ipa_dir_contains_ipa(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ipa_dir_contains_ipa_true_case_insensitive_extension() {
        let dir = temp_base_dir("ipa-dir-case-insensitive");
        fs::write(dir.join("Runner.IPA"), b"dummy ipa bytes").unwrap();
        assert!(ipa_dir_contains_ipa(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ipa_dir_contains_ipa_false_when_empty_or_missing_or_other_files() {
        let empty_dir = temp_base_dir("ipa-dir-empty");
        assert!(!ipa_dir_contains_ipa(&empty_dir));
        fs::write(empty_dir.join("notes.txt"), b"not an ipa").unwrap();
        assert!(!ipa_dir_contains_ipa(&empty_dir));
        let _ = fs::remove_dir_all(&empty_dir);

        let missing_dir = std::env::temp_dir().join(format!("bildorak-missing-ipa-dir-{}", Uuid::new_v4()));
        assert!(!ipa_dir_contains_ipa(&missing_dir));
    }

    /// 핵심 회귀 테스트 - flutter_tools 실측대로 xcodebuild -exportArchive 가 실패해도 flutter 자체는
    /// exit 0 을 낼 수 있다. "true" 로 그 상황(성공 exit code, .ipa 없음)을 흉내내 ipa 사후 검증이 실제로
    /// 상태를 Failed 로 뒤집는지 확인한다(AndroidRelease 서명 검증 테스트들과 동일한 검증 원칙).
    #[test]
    fn ios_release_without_real_ipa_is_marked_failed_even_on_exit_zero() {
        let base_dir = temp_base_dir("ios-release-no-ipa");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        let job = spawn_build_job(&base_dir, &project, BuildTarget::IosRelease, "true", &[], None)
            .expect("spawn should succeed");
        assert_eq!(job.status, BuildJobStatus::Running);

        let final_job = wait_for_finish(&base_dir, &project, Duration::from_secs(10));
        assert_eq!(
            final_job.status,
            BuildJobStatus::Failed,
            "ipa 파일이 없으면 exit 0 이어도 실패로 뒤집혀야 한다"
        );
        assert!(final_job.status_note.as_deref().unwrap_or("").contains("ipa"));

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    #[test]
    fn ios_release_with_real_ipa_file_stays_success() {
        let base_dir = temp_base_dir("ios-release-with-ipa");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        let ipa_dir = repo_dir.join("build/ios/ipa");
        fs::create_dir_all(&ipa_dir).unwrap();
        fs::write(ipa_dir.join("Runner.ipa"), b"dummy ipa bytes").unwrap();
        let project = fake_project(&repo_dir);

        let job = spawn_build_job(&base_dir, &project, BuildTarget::IosRelease, "true", &[], None)
            .expect("spawn should succeed");
        assert_eq!(job.status, BuildJobStatus::Running);
        let final_job = wait_for_finish(&base_dir, &project, Duration::from_secs(10));
        assert_eq!(
            final_job.status,
            BuildJobStatus::Success,
            ".ipa 파일이 실제로 있으면 성공으로 남아야 한다"
        );

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    // ── CLI(bildorak-cli) 지원 헬퍼(3단계) ────────────────────────────────────────────────

    #[test]
    fn read_log_from_offset_returns_empty_when_file_missing() {
        let missing = std::env::temp_dir().join(format!("bildorak-no-log-{}", Uuid::new_v4()));
        let (chunk, offset) = read_log_from_offset(&missing, 0).expect("파일 없음은 에러가 아니다");
        assert_eq!(chunk, "");
        assert_eq!(offset, 0);
    }

    #[test]
    fn read_log_from_offset_streams_appended_content_without_missing_bytes() {
        let dir = temp_base_dir("log-offset");
        let log_path = dir.join("test.log");
        fs::write(&log_path, "line1\n").unwrap();

        let (chunk, offset1) = read_log_from_offset(&log_path, 0).expect("첫 읽기 실패하면 안 된다");
        assert_eq!(chunk, "line1\n");
        assert_eq!(offset1, 6);

        // 같은 offset 으로 다시 읽으면(새 내용 없음) 빈 문자열 + 같은 offset.
        let (chunk_again, offset_again) =
            read_log_from_offset(&log_path, offset1).expect("변화 없을 때도 에러면 안 된다");
        assert_eq!(chunk_again, "");
        assert_eq!(offset_again, offset1);

        // 파일에 이어 쓰면(빌드 로그가 계속 쌓이는 상황) offset 이후 새 내용만 잡혀야 한다.
        use std::io::Write;
        let mut f = fs::OpenOptions::new().append(true).open(&log_path).unwrap();
        f.write_all(b"line2\nline3\n").unwrap();
        drop(f);

        let (chunk2, offset2) = read_log_from_offset(&log_path, offset1).expect("이어 읽기 실패하면 안 된다");
        assert_eq!(chunk2, "line2\nline3\n");
        assert_eq!(offset2, offset1 + 12);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn split_log_chunk_drops_only_trailing_newline_artifact() {
        assert_eq!(split_log_chunk(""), Vec::<String>::new());
        assert_eq!(split_log_chunk("only-line"), vec!["only-line".to_string()]);
        assert_eq!(
            split_log_chunk("line1\nline2\n"),
            vec!["line1".to_string(), "line2".to_string()]
        );
        // 로그 자체의 의도된 빈 줄은 살아남아야 한다(끝의 개행 하나만 제거 대상).
        assert_eq!(
            split_log_chunk("line1\n\nline3\n"),
            vec!["line1".to_string(), "".to_string(), "line3".to_string()]
        );
    }

    /// 테스트 전용 observer - 받은 로그 줄과 최종 job 스냅샷을 그대로 모은다(CLI 의 HumanLogObserver/
    /// JsonObserver 는 실제로 화면에 출력하지만, 여기서는 watch_build_to_completion 이 그 값들을
    /// 정확히 넘겨주는지만 확인하면 된다).
    struct CollectingObserver {
        lines: Vec<String>,
        done: Option<BuildJob>,
    }

    impl BuildObserver for CollectingObserver {
        fn on_log(&mut self, lines: &[String]) {
            self.lines.extend_from_slice(lines);
        }
        fn on_done(&mut self, job: &BuildJob) {
            self.done = Some(job.clone());
        }
    }

    #[test]
    fn watch_build_to_completion_reports_log_lines_and_final_success() {
        let base_dir = temp_base_dir("watch-success");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        // "echo" 는 즉시 끝나면서 로그 파일(stdout 리다이렉트)에 한 줄을 남긴다 - spawn_build_job 은
        // 테스트에서 항상 쓰는 고정 패턴(실제 flutter 대신 빠른 표준 커맨드) 그대로다.
        let spawned = spawn_build_job(
            &base_dir,
            &project,
            BuildTarget::AndroidDebug,
            "echo",
            &["hello-from-watch-test"],
            None,
        )
        .expect("spawn should succeed");
        assert_eq!(spawned.status, BuildJobStatus::Running);

        let mut observer = CollectingObserver { lines: Vec::new(), done: None };
        let final_job = watch_build_to_completion(
            &base_dir,
            &project,
            BuildTarget::AndroidDebug,
            &mut observer,
            Duration::from_millis(20),
        )
        .expect("watch should succeed");

        assert_eq!(final_job.status, BuildJobStatus::Success);
        assert!(
            observer.lines.iter().any(|l| l.contains("hello-from-watch-test")),
            "실제 echo 출력이 on_log 로 전달돼야 한다: {:?}",
            observer.lines
        );
        assert_eq!(observer.done.map(|j| j.status), Some(BuildJobStatus::Success));

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    #[test]
    fn watch_build_to_completion_reports_final_failure() {
        let base_dir = temp_base_dir("watch-failure");
        let repo_dir = std::env::temp_dir().join(format!("bildorak-fake-repo-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();
        let project = fake_project(&repo_dir);

        let spawned = spawn_build_job(&base_dir, &project, BuildTarget::IosSimDebug, "false", &[], None)
            .expect("spawn should succeed");
        assert_eq!(spawned.status, BuildJobStatus::Running);

        let mut observer = CollectingObserver { lines: Vec::new(), done: None };
        let final_job = watch_build_to_completion(
            &base_dir,
            &project,
            BuildTarget::IosSimDebug,
            &mut observer,
            Duration::from_millis(20),
        )
        .expect("watch should succeed even when the build itself fails");

        assert_eq!(final_job.status, BuildJobStatus::Failed);
        assert_eq!(observer.done.map(|j| j.status), Some(BuildJobStatus::Failed));

        let _ = fs::remove_dir_all(&base_dir);
        let _ = fs::remove_dir_all(&repo_dir);
    }

    #[test]
    fn cli_manifest_covers_all_six_scope_a_commands_in_order() {
        let manifest = cli_manifest();
        let names: Vec<&str> = manifest.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["apps", "build", "status", "keys", "doctor", "releases"]);
        for doc in &manifest {
            assert!(!doc.description.is_empty(), "{} 의 description 이 비어 있다", doc.name);
            assert!(!doc.example.is_empty(), "{} 의 example 이 비어 있다", doc.name);
            assert!(
                doc.example.starts_with("bildorak-cli "),
                "{} 의 example 은 실제로 타이핑할 명령이어야 한다",
                doc.name
            );
        }
    }
}
