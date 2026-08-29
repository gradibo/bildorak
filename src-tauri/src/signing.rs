// signing.rs — 서명키 등록 + 메타데이터(겉정보) 추출(출시 준비 1차 골격). 실제 배포 서명·스토어
// 업로드는 범위 밖(다음 로드맵 단계) — 여기는 등록 + 시각화 + 만료 표시까지만 한다.
//
// ⚠️ 보안 핵심(절대 위반 금지): 키의 비밀 내용(private key 바이트·비밀번호·인증서 payload)은 어디에도
// 저장·로그·반환하지 않는다 — 아래 read_cert_expiry 는 openssl 표준출력 중 "만료일" 한 줄만 취하고
// 그 외 출력(인증서 본문 등)은 아예 요청하지 않는다(-noout). signing_keys.json 에는 model.rs::
// SigningKeyRecord 그대로 — 겉정보(종류·이름·만료일)와 경로만 저장하고, 비밀번호는 오직 macOS
// keychain(아래 "Android release 서명 자동 주입" 절)에만 있다.
//
// 파일 자체 복사(확정된 설계 결정, keystore 안전 보관): Android keystore(kind == AndroidKeystore)만
// 원본을 건드리지 않고 사본을 앱 데이터 볼트(아래 copy_keystore_into_vault, commands.rs::
// register_signing_key/key_scan.rs::import_android_signing 이 호출)로 복사해 둔다 — keystore 분실 시
// 복구 불가라 백업이 목적이다. 원본은 이동·삭제·수정 절대 금지(std::fs::copy 만 쓴다 — 원본은 그대로
// 두고 사본만 만든다). iOS 인증서/API 키(cer/pem/crt/p12/p8)는 여전히 원본 위치를 참조만 하고 복사하지
// 않는다(범위 밖 — 그 종류는 keychain 비밀번호 개념이 없다).
//
// 외부 도구(openssl)는 preflight.rs/child_env.rs 와 동일한 "엔진 원칙" 을 따른다 — Command::new +
// 고정 argv 배열로만 실행하고, 사용자가 고른 경로는 인자로만 전달한다(셸 문자열 조립 금지). env 도
// child_env 의 allowlist 를 재사용해 secrets 를 상속하지 않는다.
//
// 저장은 store.rs 의 write_json_atomic(temp+rename)을 재사용한다(license.rs/build.rs 와 동일 패턴).
// projects.json 과 마찬가지로 목록이 작은 개인 데스크톱 앱이라 등록/해제/연결 모두 배열 전체를 다시
// 쓰는 단순 read-modify-write 로 충분하다(전용 파일 락 없음 — store.rs 주석과 동일 이유. commands.rs
// 가 이 파일의 함수들을 순서대로 호출하는 동기 경로뿐이라, build.rs 처럼 백그라운드 스레드가 동시에
// 같은 파일을 쓰는 경합이 없다).

use crate::child_env;
use crate::model::{AndroidSigningConfig, KeySourceInfo, SigningKeyKind, SigningKeyRecord};
use crate::store::write_json_atomic;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SIGNING_KEYS_FILE: &str = "signing_keys.json";

fn signing_keys_file_path(base_dir: &Path) -> PathBuf {
    base_dir.join(SIGNING_KEYS_FILE)
}

/// 저장된 서명키 목록을 읽는다. 파일이 없으면(첫 등록 전) 빈 목록 — store.rs::load_projects 와
/// 동일 규칙.
pub fn load_signing_keys(base_dir: &Path) -> Result<Vec<SigningKeyRecord>, String> {
    let path = signing_keys_file_path(base_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("등록된 서명키 목록을 읽지 못했어요: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&raw).map_err(|e| format!("등록된 서명키 목록이 손상됐어요: {e}"))
}

/// 목록 전체를 저장한다(pretty JSON) — store.rs::save_projects 와 동일 규칙.
pub fn save_signing_keys(base_dir: &Path, keys: &[SigningKeyRecord]) -> Result<(), String> {
    let path = signing_keys_file_path(base_dir);
    let raw = serde_json::to_string_pretty(keys)
        .map_err(|e| format!("저장할 데이터를 만들지 못했어요: {e}"))?;
    write_json_atomic(&path, &raw).map_err(|e| format!("서명키 목록을 저장하지 못했어요: {e}"))
}

/// 확장자로 종류를 감지한다(대소문자 구분 없음). 모르는 확장자는 None — 호출부(build_record)가
/// 등록 자체를 거부한다.
fn detect_kind(path: &Path) -> Option<SigningKeyKind> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    match ext.as_str() {
        "cer" | "pem" | "crt" => Some(SigningKeyKind::IosCert),
        "p8" => Some(SigningKeyKind::IosApiKey),
        "jks" | "keystore" => Some(SigningKeyKind::AndroidKeystore),
        // 암호로 보호된 인증서(대개 iOS 배포/개발 인증서를 Keychain 에서 내보낸 형식) — cer/pem/crt
        // 와 같은 IosCert 로 묶되, 만료 파싱은 아래 build_record 에서 따로 건너뛴다(암호 필요).
        "p12" => Some(SigningKeyKind::IosCert),
        _ => None,
    }
}

/// openssl 로 iOS 인증서(암호 없는 .cer/.pem/.crt) 만료일을 읽어 RFC3339 로 변환한다. 실패(도구
/// 없음/파싱 실패)는 전부 None — preflight.rs 의 "관대한 처리" 철학과 동일하게 하드 에러를 만들지
/// 않는다(build_record 가 이 None 을 그대로 받아 "확인 불가" 상태로 등록을 계속 진행한다).
///
/// 고정 argv 만 쓴다 — 사용자가 고른 경로는 인자로만 넘기고 셸을 거치지 않는다(엔진 원칙). env 는
/// child_env 의 allowlist 를 그대로 써서 secrets 를 상속하지 않는다(preflight.rs::allowlisted_command
/// 와 동일 목적). -noout 이라 인증서 본문(payload)은 애초에 표준출력에 나오지 않는다.
fn read_cert_expiry(path: &Path) -> Option<String> {
    let mut cmd = Command::new("openssl");
    child_env::apply_allowlisted_env(&mut cmd);
    cmd.args(["x509", "-enddate", "-noout", "-in"]).arg(path);
    cmd.stdin(Stdio::null());
    cmd.stderr(Stdio::null());
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_enddate(&String::from_utf8_lossy(&output.stdout))
}

/// openssl 출력(실측: `openssl x509 -enddate -noout` → "notAfter=Sep 15 05:14:35 2026 GMT\n")을
/// RFC3339 로 바꾼다. openssl 의 X509 텍스트 출력은 항상 GMT(UTC) 기준이라 고정 포맷으로 파싱한다 —
/// "notAfter=" 접두사가 없거나 "GMT" 접미사가 없으면(예상 밖 openssl 버전 등) None 으로 안전하게
/// 물러난다(하드 에러 금지, read_cert_expiry 주석과 동일 원칙).
fn parse_enddate(raw: &str) -> Option<String> {
    let line = raw.lines().next()?.trim();
    let value = line.strip_prefix("notAfter=")?.trim();
    let without_tz = value.strip_suffix("GMT")?.trim();
    let naive = chrono::NaiveDateTime::parse_from_str(without_tz, "%b %e %H:%M:%S %Y").ok()?;
    Some(naive.and_utc().to_rfc3339())
}

/// 파일 경로에서 종류를 감지하고 겉정보(만료일 등)를 추출해 새 레코드를 만든다. 저장(IO)은 하지
/// 않는다 — 호출부(commands.rs::register_signing_key)가 저장 여부까지 결정한다(pubspec.rs::
/// detect_project 가 등록까지는 안 하는 것과 동일한 "감지/파싱 vs 저장" 경계).
pub fn build_record(path: &Path) -> Result<SigningKeyRecord, String> {
    if !path.is_file() {
        return Err("선택한 파일을 찾을 수 없어요.".to_string());
    }
    let kind = detect_kind(path).ok_or_else(|| {
        "지원하지 않는 파일 형식이에요. iOS 인증서(.cer/.pem/.crt/.p12), App Store Connect API 키(.p8), \
         Android keystore(.jks/.keystore) 파일을 선택해 주세요."
            .to_string()
    })?;
    let display_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let ext_lower = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    let expires_at = match kind {
        SigningKeyKind::IosCert => {
            if ext_lower.as_deref() == Some("p12") {
                // 암호로 보호된 형식 — 1차에서는 파싱하지 않는다(detect_kind 주석과 동일 이유). 시도해도
                // 항상 실패할 openssl 호출을 아예 만들지 않는다.
                None
            } else {
                read_cert_expiry(path)
            }
        }
        // 원래 만료 개념이 없다 — "확인 불가"가 아니라 "만료 없음"(kind 로 프론트가 구분,
        // model.rs::SigningKeyRecord::expires_at 주석 참고).
        SigningKeyKind::IosApiKey => None,
        // 1차 범위 밖(비밀번호 파싱 추후) — 항상 "확인 불가".
        SigningKeyKind::AndroidKeystore => None,
    };

    Ok(SigningKeyRecord {
        id: uuid::Uuid::new_v4().to_string(),
        kind,
        display_name,
        file_path: path.to_string_lossy().to_string(),
        expires_at,
        linked_project_ids: Vec::new(),
        android_signing: None,
        // 볼트 복사는 여기서 하지 않는다 — build_record 는 "감지/파싱만"(파일 상단 detect_kind 문서
        // 참고), 실제 복사는 AppHandle(app_data_dir)이 필요해 호출부(commands.rs::register_signing_key,
        // key_scan.rs::import_android_signing)가 이 함수 호출 "다음" 에 kind == AndroidKeystore 일 때만
        // copy_keystore_into_vault 로 채운다.
        vault_path: None,
    })
}

// ── Android release 서명 자동 주입 — keychain 저장/조회 + 빌드 후 서명 검증(다음 단계) ─────────
//
// 비밀번호는 macOS Keychain 의 generic-password 항목에만 저장한다(평문 파일·JSON 저장 금지 — 확정된
// 설계 결정). 서비스 이름 규칙: "bildorak.signing.<keyId>.store" / "bildorak.signing.<keyId>.key",
// account 는 항상 key_alias 그대로(model.rs::AndroidSigningConfig 주석 참고). 저장/조회 모두 고정
// argv 의 `/usr/bin/security` 호출뿐 — 셸 조립 없음(엔진 원칙, read_cert_expiry 와 동일).
//
// ⚠️ 실측(2026-08-17, 이 머신): `security add-generic-password ... -U` 로 만든 항목은
// `security find-generic-password -w` 로 헤드리스(프롬프트 없이) 바로 읽힌다 — Keychain Access 팝업이
// 뜨지 않는다. 삭제 뒤 재조회는 exit 44("could not be found")로 실패한다. 아래 함수들은 이 실측 그대로
// 동작을 가정한다.
//
// 빌드 후 검증은 jarsigner 와 keytool 두 도구를 같이 쓴다:
//   - jarsigner -verify -verbose -certs: 서명 구조 자체가 유효한지(변조/손상 탐지) — 확정된 설계의
//     필수 1차 게이트. ⚠️ 실측: 종료 코드는 "서명 안 됨" 케이스를 구분하지 못한다(서명 안 된 파일도
//     exit 0 을 내고 stdout 문구만 "jar is unsigned." 로 다르다) — 그래서 exit code 가 아니라 stdout 의
//     "jar verified" 문구로 성공을 판정한다.
//   - keytool -printcert -jarfile / keytool -list -v: 실제 서명 인증서의 SHA-256 지문을 얻는다.
//     ⚠️ 실측: JDK 17 의 `jarsigner -verify -verbose -certs` 출력에는 인증서 SHA-256 지문 문자열이
//     아예 나오지 않는다(서명자 DN·서명 알고리즘·만료일만 나온다) — "지문이 일치하는지" 비교에 jarsigner
//     출력을 파싱하는 코드를 짜면 항상 못 찾아 깨진다. keytool 의 두 출력(-printcert -jarfile 로 빌드
//     산출물 쪽, -list -v 로 keystore 쪽)은 형식이 동일해서("\t SHA256: XX:XX:...") 직접 비교할 수
//     있다 — 확정된 설계 문구의 "keystore의 인증서 지문은 keytool -list ... 로 얻거나"를 그대로 따른
//     것이다. jarsigner 는 "구조가 유효한가"만 담당하고, "누구 인증서로 서명됐는가"는 keytool 이 담당하는
//     역할 분리로 구현했다.

const SECURITY_BIN: &str = "/usr/bin/security";

/// keychain 서비스 이름 규칙 — model.rs::AndroidSigningConfig 의 store_password_service /
/// key_password_service 를 만드는 유일한 지점(다른 곳에서 이 문자열 포맷을 다시 만들지 않는다).
fn store_password_service_name(key_id: &str) -> String {
    format!("bildorak.signing.{key_id}.store")
}

fn key_password_service_name(key_id: &str) -> String {
    format!("bildorak.signing.{key_id}.key")
}

/// macOS Keychain 에 generic-password 하나를 저장(이미 있으면 갱신, `-U`). 고정 argv만 쓴다 —
/// 비밀번호 값 자체도 인자로만 전달되고 셸을 거치지 않는다. 에러 메시지에는 절대 password 원문을
/// 넣지 않는다(위 파일 상단 "실측" 문단 및 프로젝트 보안 원칙).
fn store_keychain_password(service: &str, account: &str, password: &str) -> Result<(), String> {
    let mut cmd = Command::new(SECURITY_BIN);
    child_env::apply_allowlisted_env(&mut cmd);
    cmd.args(["add-generic-password", "-s", service, "-a", account, "-w", password, "-U"]);
    cmd.stdin(Stdio::null());
    let output = cmd
        .output()
        .map_err(|e| format!("비밀번호를 keychain에 저장하지 못했어요: {e}"))?;
    if !output.status.success() {
        return Err("비밀번호를 macOS 키체인에 저장하지 못했어요.".to_string());
    }
    Ok(())
}

/// macOS Keychain 에서 generic-password 하나를 읽는다. stdout 은 비밀번호 원문 그대로라(위
/// "실측" 문단) 이 함수 밖으로는 Result<String,..> 로만 반환하고, 실패 시 에러 문구에 stdout/stderr
/// 원문을 절대 포함하지 않는다. build.rs::resolve_android_signing 이 release 빌드 시작 직전에
/// 호출한다(pub) — 반환된 String 은 -P 빌드 인자로만 쓰이고 로그/에러에 다시 넣지 않는다.
pub fn read_keychain_password(service: &str, account: &str) -> Result<String, String> {
    let mut cmd = Command::new(SECURITY_BIN);
    child_env::apply_allowlisted_env(&mut cmd);
    cmd.args(["find-generic-password", "-s", service, "-a", account, "-w"]);
    cmd.stdin(Stdio::null());
    let output = cmd
        .output()
        .map_err(|e| format!("비밀번호를 keychain에서 불러오지 못했어요: {e}"))?;
    if !output.status.success() {
        return Err(
            "등록된 서명 비밀번호를 키체인에서 찾지 못했어요. 서명키 비밀번호를 다시 등록해 주세요."
                .to_string(),
        );
    }
    let password = String::from_utf8_lossy(&output.stdout)
        .trim_end_matches(['\n', '\r'])
        .to_string();
    if password.is_empty() {
        return Err(
            "키체인에 저장된 비밀번호가 비어 있어요. 서명키 비밀번호를 다시 등록해 주세요.".to_string(),
        );
    }
    Ok(password)
}

/// keychain 항목 삭제 — best-effort(이미 없어도 조용히 무시). 서명키 삭제(remove_signing_key)나
/// alias 변경으로 이전 keychain 항목이 고아가 될 때 정리하는 용도(commands.rs 가 호출).
fn delete_keychain_password(service: &str, account: &str) {
    let mut cmd = Command::new(SECURITY_BIN);
    child_env::apply_allowlisted_env(&mut cmd);
    cmd.args(["delete-generic-password", "-s", service, "-a", account]);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    let _ = cmd.status();
}

/// keychain 항목 삭제(공개판) — commands.rs 가 remove_signing_key / register_android_signing(alias
/// 변경) 에서 고아 항목을 정리할 때 쓴다.
pub fn forget_android_signing_secrets(config: &AndroidSigningConfig) {
    delete_keychain_password(&config.store_password_service, &config.keychain_account);
    delete_keychain_password(&config.key_password_service, &config.keychain_account);
}

/// keystore 원본을 안전 보관 볼트(vault_dir, store.rs::keystore_vault_dir 가 app_data_dir 하위에 만들어
/// 둔 폴더)로 복사한다 — 이동이 아니라 복사이며 원본은 절대 손대지 않는다(std::fs::copy 는 원본을
/// 그대로 둔 채 사본만 만든다, 확정된 설계 "이동하면 프로젝트 build config 가 깨진다"). 파일명이
/// 겹쳐도 안전하도록 "<key_id>-<원본 파일명>"으로 저장한다(서로 다른 두 프로젝트가 우연히 같은 이름의
/// keystore 를 등록해도 볼트 안에서 서로 덮어쓰지 않는다). 복사 자체가 실패하면(디스크 공간 부족 등)
/// 그대로 Err — 볼트 백업이 이 기능의 핵심 안전장치라 조용히 건너뛰지 않는다(keychain 저장 실패를
/// 하드 에러로 막는 store_keychain_password 와 동일 원칙).
///
/// ⚠️ 실측(사용자 보고): 원본이 클라우드(구글드라이브 등) online-only 상태(macOS File Provider — Finder 에
/// 다운로드 클라우드 아이콘으로 표시)면 단순 std::fs::copy 는 온디맨드 다운로드를 기다리다 "Operation
/// timed out (os error 60)"(ETIMEDOUT)로 실패한다. 그래서 두 단계로 나눈다:
///   1) 최종 이름이 아니라 임시본(<dest>.part)에 먼저 복사한다. copy_with_retry 가 ETIMEDOUT 류 일시적
///      에러만 고정 백오프(RETRY_BACKOFF_SECS)로 재시도한다 — 첫 접근이 백그라운드 다운로드를 촉발하고
///      재시도 사이 대기하면 그새 다운로드가 끝나 다음 시도가 성공한다(실측). 이 함수는 register 계열
///      커맨드가 spawn_blocking 워커 스레드에서만 호출하므로(commands.rs::register_signing_key,
///      key_scan.rs::import_android_signing) std::thread::sleep 로 블로킹해도 async 런타임을 막지 않는다.
///   2) 복사가 성공하면 임시본 크기를 검증한 뒤(0바이트거나 원본 크기와 다르면 신뢰하지 않는다) 같은
///      볼트 폴더 안에서 fs::rename 으로 최종 이름으로 바꾼다(같은 파일시스템 내 이동이라 원자적 —
///      볼트 안에는 "복사 중" 상태의 파일이 아니라 완전한 최종본만 보인다). 끝까지 실패하면 임시본은
///      best-effort 로 지운다 — 볼트 폴더에 부분 복사본이 남지 않는다. 원본은 이 흐름 전체에서 한 번도
///      쓰기 대상이 아니다(위 문단과 동일 불변조건, read-only 접근만 한다).
pub fn copy_keystore_into_vault(vault_dir: &Path, keystore_path: &Path, key_id: &str) -> Result<PathBuf, String> {
    let file_name = keystore_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "keystore".to_string());
    let dest = vault_dir.join(format!("{key_id}-{file_name}"));
    let tmp_dest = vault_dir.join(format!("{key_id}-{file_name}.part"));
    // 재시도 사이 원본이 바뀔 걱정은 없다(원본은 read-only) — 복사 시작 전 한 번만 크기를 읽어 성공
    // 판정 기준으로 쓴다. 못 읽어도(권한 등) 하드 에러로 만들지 않는다 — 그때는 ">0" 만으로 판정한다
    // (copy_into_vault_with).
    let source_len = std::fs::metadata(keystore_path).ok().map(|m| m.len());
    let tmp_dest_for_copy = tmp_dest.clone();
    copy_into_vault_with(
        &RETRY_BACKOFF_SECS,
        move || std::fs::copy(keystore_path, &tmp_dest_for_copy),
        keystore_path,
        &tmp_dest,
        &dest,
        source_len,
    )
}

/// 클라우드 파일 온디맨드 다운로드를 기다리는 재시도 스케줄(초 단위, 최초 시도 다음부터 순서대로
/// 대기) — 총 6회 재시도(최초 시도까지 최대 7회 시도), 대기 합 31초 상한. 프로덕션 경로
/// (copy_keystore_into_vault)만 이 상수를 쓴다 — copy_with_retry/copy_into_vault_with 는 delays 를
/// 인자로 받아 테스트가 대기 없는(0초) 스케줄을 대신 넣을 수 있게 한다.
const RETRY_BACKOFF_SECS: [u64; 6] = [1, 2, 4, 8, 8, 8];

/// ETIMEDOUT(macOS 에서 raw os error 60 — 클라우드 온디맨드 다운로드 대기 중 실측: "Operation timed out
/// (os error 60)")이거나 std::io::ErrorKind::TimedOut/Interrupted 면 재시도 대상. 권한 없음·디스크 공간
/// 부족 등 재시도해도 결과가 안 바뀔 에러는 즉시 포기한다(불필요하게 최대 31초를 붙잡지 않는다).
fn is_retryable_io_error(err: &std::io::Error) -> bool {
    err.raw_os_error() == Some(60)
        || matches!(err.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted)
}

/// "복사 시도 1회"를 표현하는 클로저(attempt)를 받아 delays 스케줄대로 재시도한다 — 재시도 대상이
/// 아닌 에러는 첫 실패에서 바로 반환한다. attempt 를 클로저로 받는 이유는 테스트가 실제 fs::copy 대신
/// "N 번 실패 후 성공"/"끝까지 실패" 를 흉내낸 가짜 클로저를 주입할 수 있게 하기 위해서다.
fn copy_with_retry(delays: &[u64], mut attempt: impl FnMut() -> std::io::Result<u64>) -> std::io::Result<u64> {
    let mut result = attempt();
    for delay in delays {
        let retryable = matches!(&result, Err(e) if is_retryable_io_error(e));
        if !retryable {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(*delay));
        result = attempt();
    }
    result
}

/// 클라우드 저장소 온디맨드 폴더의 고정 경로 세그먼트 — Apple(iCloud Drive/CloudStorage)과 주요 동기화
/// 클라이언트(Dropbox/OneDrive)가 로케일과 무관하게 쓰는 폴더명이다(사용자가 지정하는 표시 이름이
/// 아니라 파일시스템 경로 자체라 안정적으로 매칭할 수 있다).
const CLOUD_STORAGE_MARKERS: [&str; 5] =
    ["/Library/CloudStorage/", "com~apple~CloudDocs", "Mobile Documents", "Dropbox", "OneDrive"];

/// pub — commands.rs::inspect_key_source 도 이 판정을 그대로 재사용한다(같은 마커 목록을 두 곳에
/// 중복 구현하지 않는다).
pub fn looks_like_cloud_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    CLOUD_STORAGE_MARKERS.iter().any(|marker| path_str.contains(marker))
}

/// copy_with_retry 가 끝까지 실패했을 때 사용자에게 보여줄 메시지 — 원본 경로가 클라우드 온디맨드
/// 폴더처럼 보이면(looks_like_cloud_path) 안내 문구를 덧붙이고, 아니면 기존 메시지를 그대로 쓴다.
fn build_copy_error_message(keystore_path: &Path, err: &std::io::Error) -> String {
    let base = format!("keystore 파일을 안전 보관 폴더에 복사하지 못했어요: {err}");
    if looks_like_cloud_path(keystore_path) {
        format!(
            "{base}\n이 파일이 클라우드(구글드라이브/iCloud 등)에 있어요. Finder에서 먼저 다운로드하거나 \
             '오프라인에서 사용 가능'으로 표시한 뒤 다시 시도하세요."
        )
    } else {
        base
    }
}

/// copy_keystore_into_vault 의 실제 IO 처리부 — attempt(복사 시도 1회)와 backoff 스케줄(delays)을 인자로
/// 받아 분리했다. 테스트가 실제 fs::copy·클라우드 파일 없이도 "재시도 후 성공"과 "끝까지 실패 시 임시본
/// 정리"를 대기 없이(delays 에 0 을 넣어) 검증할 수 있다 — 공개 함수는 이 함수를 실제 std::fs::copy
/// 클로저 + RETRY_BACKOFF_SECS 로 호출하는 얇은 래퍼일 뿐이다.
fn copy_into_vault_with(
    delays: &[u64],
    attempt: impl FnMut() -> std::io::Result<u64>,
    keystore_path: &Path,
    tmp_dest: &Path,
    dest: &Path,
    source_len: Option<u64>,
) -> Result<PathBuf, String> {
    let copied_len = match copy_with_retry(delays, attempt) {
        Ok(len) => len,
        Err(e) => {
            let _ = std::fs::remove_file(tmp_dest); // best-effort — 부분 복사본을 볼트에 남기지 않는다.
            return Err(build_copy_error_message(keystore_path, &e));
        }
    };

    // 0바이트거나(온디맨드 다운로드가 중간에 조용히 끊긴 경우 등) 원본 크기를 알아낼 수 있었는데 그
    // 값과 다르면(가능하면 비교 — 위 copy_keystore_into_vault 참고) 임시본을 신뢰하지 않는다.
    let size_ok = copied_len > 0 && source_len.map(|len| len == copied_len).unwrap_or(true);
    if !size_ok {
        let _ = std::fs::remove_file(tmp_dest);
        return Err(
            "keystore 파일을 복사했지만 크기가 원본과 달라요(다운로드가 끝나기 전에 복사가 끝났을 수 \
             있어요). 다시 시도해 주세요."
                .to_string(),
        );
    }

    std::fs::rename(tmp_dest, dest).map_err(|e| {
        let _ = std::fs::remove_file(tmp_dest);
        format!("복사한 keystore 파일을 최종 위치로 옮기지 못했어요: {e}")
    })?;
    Ok(dest.to_path_buf())
}

// ── 클라우드 키 선제 알림(등록 전 사전 확인) ────────────────────────────────────────────────
// "등록"/"가져오기" 버튼을 누른 직후, copy_keystore_into_vault(위)가 최대 ~31초 재시도하다 실패하는
// 것을 사용자가 매번 기다리지 않도록 미리 판정한다(리뷰 지적 — 재시도로 헛돌지 않고 즉시 안내).
// commands.rs::inspect_key_source 가 이 함수들을 그대로 노출한다.

/// looks_like_cloud_path 가 잡아낸 마커 중 사람이 읽을 이름으로 매핑한다 — 확정된 라벨 4가지만
/// 명시로 반환한다(Google Drive/iCloud/Dropbox/OneDrive). 그 외 CloudStorage 통합 제공자(예: Box)는
/// 이름을 추측하지 않고 None — 프론트가 일반 문구로 대신 표시한다(copy.ts::cloudKindLabelFromPath
/// 문서 참고).
fn cloud_kind_label(path: &Path) -> Option<String> {
    let path_str = path.to_string_lossy();
    if path_str.contains("com~apple~CloudDocs") || path_str.contains("Mobile Documents") {
        Some("iCloud".to_string())
    } else if path_str.contains("GoogleDrive") {
        Some("Google Drive".to_string())
    } else if path_str.contains("Dropbox") {
        Some("Dropbox".to_string())
    } else if path_str.contains("OneDrive") {
        Some("OneDrive".to_string())
    } else {
        None
    }
}

/// 온디맨드(placeholder) 파일인지 실측 판정 — macOS 통합 클라우드 저장소(iCloud/Google Drive/Dropbox/
/// OneDrive 전부 File Provider 기반)는 아직 내려받지 않은 파일도 크기(len)는 실제 값 그대로 보고하지만
/// 디스크 블록은 할당하지 않는다(len>0 인데 blocks==0). stat(2) 결과만 보고 파일을 열거나 읽지 않으므로
/// 온디맨드 다운로드를 유발하지 않는다(materialize 없음 — 위 copy_keystore_into_vault 의 재시도/최대
/// ~31초 대기와 달리 이 함수는 즉시 반환된다). metadata 를 못 읽으면(권한 등, 드묾) 보수적으로
/// "다운로드됨"으로 본다 — 다운로드 안내를 잘못 띄우는 것보다 기존 등록 흐름(클릭 시 재시도)으로
/// 조용히 넘어가는 편이 안전하다(하드 에러 아님, 이 파일 전체의 관대한 처리 철학과 동일).
#[cfg(unix)]
fn is_file_downloaded(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(metadata) = std::fs::metadata(path) else { return true };
    !(metadata.len() > 0 && metadata.blocks() == 0)
}

#[cfg(not(unix))]
fn is_file_downloaded(_path: &Path) -> bool {
    // Windows 확장 시점에 재검토(OS 추상화 원칙, child_env.rs::kill_process_group 과 동일
    // 시접) — 지금은 온디맨드 판정 없이 "항상 다운로드됨"으로 물러난다(하드 에러 아님).
    true
}

/// 등록("가져오기") 시도 전에 원본 파일이 클라우드 온디맨드 상태인지 미리 확인한다(commands.rs::
/// inspect_key_source 가 그대로 노출) — stat 만 쓰고 파일을 열지 않는다. 존재하지 않는 경로는
/// build_record 와 동일하게 에러로 알린다.
pub fn inspect_key_source(path: &Path) -> Result<KeySourceInfo, String> {
    if !path.is_file() {
        return Err("선택한 파일을 찾을 수 없어요.".to_string());
    }
    let is_cloud = looks_like_cloud_path(path);
    let folder_name = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(KeySourceInfo {
        is_cloud,
        is_downloaded: is_file_downloaded(path),
        folder_name,
        cloud_kind: if is_cloud { cloud_kind_label(path) } else { None },
    })
}

/// key_id + key_alias 로 AndroidSigningConfig 를 만들고 두 비밀번호를 keychain 에 저장한다. keystore_path
/// 로 등록 시점 인증서 겉정보(만료일/SHA-256, 둘 다 비밀 아님)도 함께 뽑아 config 에 얹는다 —
/// import_found_android_signing(자동 이관)과 commands.rs::register_android_signing(수동 폼) 둘 다 이
/// 함수 하나만 거치므로 별도 분기 없이 공통으로 채워진다. 이 메타데이터 추출은 best-effort 라
/// keystore_path 가 없거나 keytool 이 실패해도 등록 자체(비밀번호 저장)는 계속 진행한다(read_android_
/// keystore_cert_metadata 문서 참고). 저장만 하고 signing_keys.json 갱신은 하지 않는다(commands.rs::
/// register_android_signing 이 호출부에서 레코드를 찾고/검증하고/저장까지 담당 — build_record 가 IO 는
/// 안 하고 감지/파싱만 하는 것과 같은 "감지 vs 저장" 경계, 파일 상단 주석 참고).
pub fn register_android_signing(
    keystore_path: &Path,
    key_id: &str,
    key_alias: &str,
    store_password: &str,
    key_password: &str,
) -> Result<AndroidSigningConfig, String> {
    let alias = key_alias.trim();
    if alias.is_empty() {
        return Err("키 별칭(alias)을 입력해 주세요.".to_string());
    }
    if store_password.is_empty() || key_password.is_empty() {
        return Err("저장소 비밀번호와 키 비밀번호를 모두 입력해 주세요.".to_string());
    }
    let (cert_sha256, cert_expiry) = read_android_keystore_cert_metadata(keystore_path, alias, store_password);
    let config = AndroidSigningConfig {
        key_alias: alias.to_string(),
        keychain_account: alias.to_string(),
        store_password_service: store_password_service_name(key_id),
        key_password_service: key_password_service_name(key_id),
        cert_expiry,
        cert_sha256,
    };
    store_keychain_password(&config.store_password_service, &config.keychain_account, store_password)?;
    store_keychain_password(&config.key_password_service, &config.keychain_account, key_password)?;
    Ok(config)
}

/// keytool -printcert -jarfile 또는 -list -v 출력에서 "SHA256: XX:XX:..." 줄을 찾아 지문만 뽑는다.
/// 실측(2026-08-17): 두 출력 모두 "\t SHA256: 45:83:87:68:..." 형태로 동일하다 — 시스템 로케일에 따라
/// 앞뒤 문구(예: "소유자:"/"Owner:")는 달라져도 "SHA256:" 라벨 자체는 번역되지 않는다(JDK 상수).
/// 서명이 안 됐거나("서명된 jar 파일이 아닙니다." 류) 파싱 대상이 없으면 None — 하드 에러로 만들지
/// 않는다(호출부가 검증 실패로 처리, read_cert_expiry 와 동일 원칙).
fn extract_sha256_fingerprint(output: &str) -> Option<String> {
    for line in output.lines() {
        if let Some(idx) = line.find("SHA256:") {
            let fingerprint = line[idx + "SHA256:".len()..].trim();
            if !fingerprint.is_empty() {
                return Some(fingerprint.to_uppercase());
            }
        }
    }
    None
}

/// `keytool -list -v -keystore <path> -alias <alias> -storepass <pw>` 출력 원문을 그대로 돌려준다.
/// keystore_certificate_sha256(빌드 후 검증, 실패하면 하드 에러)과 read_android_keystore_cert_metadata
/// (등록 시점, 실패해도 조용히 None) 둘 다 이 한 곳만 부른다 — keytool 호출 로직을 두 곳에 중복
/// 구현하지 않는다. 성공/실패 판정은 호출부가 한다(여기선 텍스트만).
fn run_keystore_list_v(keystore_path: &Path, alias: &str, store_password: &str) -> Result<String, String> {
    let keytool = child_env::resolve_jdk_tool("keytool")
        .ok_or_else(|| "keytool을 찾지 못했어요(JDK 설치를 확인해 주세요).".to_string())?;
    let mut cmd = Command::new(&keytool);
    child_env::apply_allowlisted_env(&mut cmd);
    cmd.args(["-list", "-v", "-keystore"]).arg(keystore_path);
    cmd.args(["-alias", alias, "-storepass", store_password]);
    cmd.stdin(Stdio::null());
    let output = cmd
        .output()
        .map_err(|e| format!("keystore 인증서 정보를 확인하지 못했어요: {e}"))?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 등록된 keystore 파일 자체의 인증서 SHA-256 지문 — `keytool -list -v` 로 얻는다. 실측 확인: 인증서
/// "겉정보"를 나열하는 -list -v 는 store 비밀번호만 있으면 되고 key 비밀번호는 필요 없다.
fn keystore_certificate_sha256(keystore_path: &Path, alias: &str, store_password: &str) -> Result<String, String> {
    let text = run_keystore_list_v(keystore_path, alias, store_password)?;
    extract_sha256_fingerprint(&text)
        .ok_or_else(|| "keystore 인증서 지문을 확인하지 못했어요(비밀번호나 별칭을 확인해 주세요).".to_string())
}

/// keytool 이 요일 약어로 시작하는 Java `Date::toString()` 고정 포맷(`EEE MMM dd HH:mm:ss zzz yyyy`,
/// 예: "Sat Aug 16 01:43:17 KST 2036")을 쓰는지 판별하는 첫 토큰 검사 — 요일 약어는 Java 언어 스펙상
/// JVM 로케일과 무관하게 항상 이 영문 3글자다(extract_sha256_fingerprint 의 "SHA256:" 라벨이 로케일과
/// 무관한 것과 같은 이유).
fn is_weekday_abbrev(token: &str) -> bool {
    matches!(token, "Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat" | "Sun")
}

fn is_month_abbrev(token: &str) -> bool {
    matches!(
        token,
        "Jan" | "Feb" | "Mar" | "Apr" | "May" | "Jun" | "Jul" | "Aug" | "Sep" | "Oct" | "Nov" | "Dec"
    )
}

/// `keytool -list -v` 전체 출력에서 Java `Date::toString()` 고정 포맷 값을 등장 순서대로 뽑는다. 실측
/// (2026-08-19, 이 머신, 한국어 로케일 JDK): "적합한 시작 날짜: <A> 종료 날짜: <B>" 처럼 한 줄에 시작일/
/// 종료일 두 값이 나온다 — 라벨 문구는 로케일에 따라 달라지지만(영문 JDK 는 "Valid from: <A> until: <B>")
/// 날짜 값 자체(요일+월 영문 약어)는 Java 스펙상 고정이라 라벨을 보지 않고 값 패턴만으로 찾는다. 다섯
/// 번째 토큰(시간대 약어, 예: KST)은 이 함수에서 아예 읽지 않는다 — parse_keystore_validity_end 가 그
/// 값을 신뢰하지 않고 대신 chrono::Local(이 keytool 을 실행한 바로 그 macOS 의 현재 시스템 시간대)로
/// 해석하기 때문이다(시간대 약어 매핑 표를 따로 유지할 필요가 없다).
fn extract_keytool_java_dates(output: &str) -> Vec<chrono::NaiveDateTime> {
    let tokens: Vec<&str> = output.split_whitespace().collect();
    let mut dates = Vec::new();
    let mut i = 0;
    while i + 6 <= tokens.len() {
        if is_weekday_abbrev(tokens[i]) && is_month_abbrev(tokens[i + 1]) {
            let combined = format!("{} {} {} {}", tokens[i + 1], tokens[i + 2], tokens[i + 3], tokens[i + 5]);
            if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(&combined, "%b %d %H:%M:%S %Y") {
                dates.push(naive);
                i += 6;
                continue;
            }
        }
        i += 1;
    }
    dates
}

/// keystore 인증서의 만료일(RFC3339) — 자체 서명 keystore(인증서 체인 길이 1, Android 서명키의 사실상
/// 전부)는 extract_keytool_java_dates 가 정확히 2개(시작일/종료일)를 순서대로 뽑고, 두 번째가 만료일
/// 이다. 체인이 더 길어 값이 더 나와도 첫 인증서(맨 앞 두 개)만 본다 — Android 릴리스 keystore 는
/// 실사용상 항상 자체 서명이라 이 범위로 충분하다(가정 — 설계 노트로 남김). 못 찾거나 파싱에 실패하면
/// None — 하드 에러로 만들지 않는다(read_cert_expiry 와 동일 철학).
fn parse_keystore_validity_end(output: &str) -> Option<String> {
    use chrono::TimeZone;
    let dates = extract_keytool_java_dates(output);
    let end = dates.get(1)?;
    let local = chrono::Local.from_local_datetime(end).single()?;
    Some(local.with_timezone(&chrono::Utc).to_rfc3339())
}

/// Android 서명 등록 시점의 인증서 겉정보(비밀 아님) 추출 — SHA-256 지문은 keystore_certificate_sha256
/// 과 같은 keytool -list -v 출력을 재사용(run_keystore_list_v 공유, 중복 호출 없음)하고, 만료일은 같은
/// 출력에서 parse_keystore_validity_end 로 새로 뽑는다. keytool 이 없거나 keystore_path 가 더는
/// 존재하지 않거나 별칭/비밀번호가 실제 keystore 와 안 맞아도(register_android_signing 자체는 값이
/// 비어 있지 않은지만 검증하고 실제 keystore 매칭까지는 확인하지 않는다) 하드 에러로 만들지 않고
/// (None, None) 을 돌려준다 — 등록 자체는 이 메타데이터 없이도 성공해야 한다(제품 요구사항).
fn read_android_keystore_cert_metadata(
    keystore_path: &Path,
    alias: &str,
    store_password: &str,
) -> (Option<String>, Option<String>) {
    let Ok(text) = run_keystore_list_v(keystore_path, alias, store_password) else {
        return (None, None);
    };
    (extract_sha256_fingerprint(&text), parse_keystore_validity_end(&text))
}

/// 실제 빌드된 산출물(aab)을 누가 서명했는지의 인증서 SHA-256 지문 — `keytool -printcert -jarfile` 로
/// 얻는다. ⚠️ 실측: 서명 안 된 파일도 종료 코드는 0 이고 "서명된 jar 파일이 아닙니다." 라고만 출력한다
/// — 그래서 종료 코드가 아니라 SHA256 줄이 실제로 있는지로 판정한다(extract_sha256_fingerprint 주석
/// 참고).
fn built_artifact_certificate_sha256(artifact_path: &Path) -> Result<String, String> {
    let keytool = child_env::resolve_jdk_tool("keytool")
        .ok_or_else(|| "keytool을 찾지 못했어요(JDK 설치를 확인해 주세요).".to_string())?;
    let mut cmd = Command::new(&keytool);
    child_env::apply_allowlisted_env(&mut cmd);
    cmd.args(["-printcert", "-jarfile"]).arg(artifact_path);
    cmd.stdin(Stdio::null());
    let output = cmd
        .output()
        .map_err(|e| format!("빌드 산출물의 서명 인증서를 확인하지 못했어요: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    extract_sha256_fingerprint(&text)
        .ok_or_else(|| "빌드 산출물이 서명되지 않았거나 서명 인증서를 확인할 수 없어요.".to_string())
}

/// `jarsigner -verify -verbose -certs` 로 서명 구조 자체가 유효한지 확인(변조/손상 탐지) — 확정된
/// 설계의 필수 1차 게이트. ⚠️ 실측: 종료 코드만으로는 "서명 안 됨" 을 못 걸러낸다(그 경우도 exit 0)
/// — stdout 에 "jar verified" 문구가 있는지로 판정한다.
fn verify_jar_signature_structure(artifact_path: &Path) -> Result<(), String> {
    let jarsigner = child_env::resolve_jdk_tool("jarsigner")
        .ok_or_else(|| "jarsigner를 찾지 못했어요(JDK 설치를 확인해 주세요).".to_string())?;
    let mut cmd = Command::new(&jarsigner);
    child_env::apply_allowlisted_env(&mut cmd);
    cmd.args(["-verify", "-verbose", "-certs"]).arg(artifact_path);
    cmd.stdin(Stdio::null());
    let output = cmd
        .output()
        .map_err(|e| format!("서명 구조를 확인하지 못했어요: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    if text.contains("jar verified") {
        Ok(())
    } else {
        Err("빌드 산출물의 서명 구조 검증에 실패했어요.".to_string())
    }
}

/// Android release 빌드가 끝난 뒤 등록 keystore 로 실제 서명됐는지 확인하는 공개 진입점(build.rs 가
/// 부른다) — injected-signing 이 조용히 무시되고 debug 키로 서명되는 사고를 막는 마지막 게이트
/// (Play 반려 실증 사례, 확정된 설계). 아래 중 하나라도 걸리면 Err:
///   1) jarsigner 서명 구조 검증 실패(변조/손상)
///   2) 산출물의 서명 인증서 지문을 못 얻음(서명 안 됨 포함)
///   3) 그 지문이 등록 keystore 의 인증서 지문과 다름(=debug 키 등 다른 키로 서명됨)
/// 반환하는 Err 문구는 기술적 사유만 담는다 — 사용자용 "서명이 안 맞아요, 스토어에 못 올려요" 문구는
/// 호출부(build.rs)가 앞에 붙인다(확정된 설계 문구 그대로 한 곳에서만 관리).
pub fn verify_release_signing(
    artifact_path: &Path,
    keystore_path: &Path,
    alias: &str,
    store_password: &str,
) -> Result<(), String> {
    verify_jar_signature_structure(artifact_path)?;
    let artifact_fingerprint = built_artifact_certificate_sha256(artifact_path)?;
    let keystore_fingerprint = keystore_certificate_sha256(keystore_path, alias, store_password)?;
    if artifact_fingerprint != keystore_fingerprint {
        return Err(
            "등록한 keystore의 인증서와 실제 서명 인증서가 달라요(다른 키로 서명됐을 수 있어요)."
                .to_string(),
        );
    }
    Ok(())
}

// ── iOS 배포 인증서 Team ID 조회(export 설정 폴백, build.rs::resolve_ios_team_id 가 호출) ────────
// project.pbxproj 에 DEVELOPMENT_TEAM 이 없는 프로젝트(이 머신 실측)를 위한 폴백 — keychain 에
// 이미 설치된 "Apple Distribution" 배포 인증서에서 Team ID 를 읽는다. 인증서를 새로 만들거나 바꾸지
// 않는다 — 이미 설치된 identity 목록을 조회(-v -p codesigning)만 하는 read-only 호출이다.

/// `security find-identity -v -p codesigning` 출력에서 첫 번째 "Apple Distribution:" 줄의 Team ID 를
/// 뽑는다. 형식은 항상 `"Apple Distribution: <이름> (<TEAMID>)"` 라 마지막 괄호 안 값을 그대로 쓰면
/// 된다(이름 자체에 괄호가 들어갈 일은 없다 — Apple 이 내려주는 고정 포맷). "Apple Development:" 줄은
/// 앱스토어 제출용이 아니므로 건너뛴다. 여러 개가 유효해도 첫 번째만 쓴다 — 어느 걸 우선할지 정할
/// 근거가 없어 임의로 특정 짓지 않는다. 괄호를 못 찾는(예상 밖 포맷) 줄은 하드 에러 대신 다음 줄로
/// 넘어간다(`?` 대신 continue — 한 줄이 예상과 달라도 나머지 줄 탐색을 막지 않는다).
fn parse_distribution_team_id(output: &str) -> Option<String> {
    for line in output.lines() {
        if !line.contains("Apple Distribution:") {
            continue;
        }
        let Some(open) = line.rfind('(') else { continue };
        let Some(close) = line[open..].find(')') else { continue };
        let team_id = line[open + 1..open + close].trim();
        if !team_id.is_empty() {
            return Some(team_id.to_string());
        }
    }
    None
}

/// keychain 에 설치된 배포 인증서에서 Team ID 를 조회한다(read_cert_expiry 와 동일한 "고정 argv + env
/// allowlist" 엔진 원칙, SECURITY_BIN 재사용). 도구 실행 자체가 실패하거나(권한 등) 유효한 배포
/// 인증서가 하나도 없으면 None — 하드 에러가 아니다(호출부 build.rs::resolve_ios_team_id 가 이 None 을
/// "둘 다 실패" 에러 메시지로 안내한다).
pub fn find_distribution_team_id() -> Option<String> {
    let mut cmd = Command::new(SECURITY_BIN);
    child_env::apply_allowlisted_env(&mut cmd);
    cmd.args(["find-identity", "-v", "-p", "codesigning"]);
    cmd.stdin(Stdio::null());
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_distribution_team_id(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bildorak-signing-test-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("failed to create temp dir");
        dir
    }

    #[test]
    fn detect_kind_matches_known_extensions_case_insensitive() {
        assert_eq!(detect_kind(Path::new("a.cer")), Some(SigningKeyKind::IosCert));
        assert_eq!(detect_kind(Path::new("a.PEM")), Some(SigningKeyKind::IosCert));
        assert_eq!(detect_kind(Path::new("a.crt")), Some(SigningKeyKind::IosCert));
        assert_eq!(detect_kind(Path::new("a.p12")), Some(SigningKeyKind::IosCert));
        assert_eq!(detect_kind(Path::new("a.P8")), Some(SigningKeyKind::IosApiKey));
        assert_eq!(detect_kind(Path::new("a.jks")), Some(SigningKeyKind::AndroidKeystore));
        assert_eq!(detect_kind(Path::new("a.keystore")), Some(SigningKeyKind::AndroidKeystore));
        assert_eq!(detect_kind(Path::new("a.txt")), None);
        assert_eq!(detect_kind(Path::new("a")), None);
    }

    #[test]
    fn parse_enddate_reads_real_openssl_output_format() {
        // 실측 캡처(openssl 3.6.3, macOS): `openssl x509 -enddate -noout` 출력 원문 그대로.
        let raw = "notAfter=Sep 15 05:14:35 2026 GMT\n";
        let parsed = parse_enddate(raw).expect("실측 포맷을 파싱하지 못했어요");
        let dt = chrono::DateTime::parse_from_rfc3339(&parsed).expect("RFC3339 여야 프론트 Date.parse 가 안전");
        assert_eq!(dt.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-09-15 05:14:35");
    }

    #[test]
    fn parse_enddate_rejects_unexpected_format() {
        assert_eq!(parse_enddate("not a date line\n"), None);
        assert_eq!(parse_enddate(""), None);
    }

    #[test]
    fn build_record_missing_file_is_error() {
        let dir = temp_dir("missing");
        let result = build_record(&dir.join("nope.cer"));
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_record_unknown_extension_is_error() {
        let dir = temp_dir("unknown-ext");
        let path = dir.join("notes.txt");
        fs::write(&path, "hello").unwrap();
        let result = build_record(&path);
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_record_ios_api_key_has_no_expiry() {
        let dir = temp_dir("p8");
        let path = dir.join("AuthKey_ABC123.p8");
        // 테스트용 더미 바이트 — 실제 App Store Connect API 키가 아니다.
        fs::write(&path, "-----BEGIN PRIVATE KEY-----\ndummy\n-----END PRIVATE KEY-----\n").unwrap();
        let record = build_record(&path).expect("등록 실패하면 안 된다");
        assert_eq!(record.kind, SigningKeyKind::IosApiKey);
        assert_eq!(record.expires_at, None);
        assert_eq!(record.display_name, "AuthKey_ABC123.p8");
        assert!(record.linked_project_ids.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_record_android_keystore_registers_without_expiry() {
        let dir = temp_dir("jks");
        let path = dir.join("release.jks");
        fs::write(&path, b"not a real keystore, just bytes for the test").unwrap();
        let record = build_record(&path).expect("등록 실패하면 안 된다");
        assert_eq!(record.kind, SigningKeyKind::AndroidKeystore);
        assert_eq!(record.expires_at, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_record_password_protected_p12_skips_parsing_without_error() {
        let dir = temp_dir("p12");
        let path = dir.join("distribution.p12");
        fs::write(&path, b"not a real p12, just bytes for the test").unwrap();
        let record = build_record(&path).expect("등록 실패하면 안 된다(파싱 불가여도 등록은 되어야 함)");
        assert_eq!(record.kind, SigningKeyKind::IosCert);
        assert_eq!(record.expires_at, None, ".p12 는 1차에서 만료를 파싱하지 않는다(확인 불가로 표시)");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 실제 openssl 파이프라인 e2e — 테스트 전용 자체 서명 인증서를 그 자리에서 생성한다(실키 절대
    /// 사용 안 함, 보안 원칙). 이 머신에 openssl 이 없으면(드묾) 조용히 건너뛴다 —
    /// preflight.rs 의 "도구 없음은 하드 에러 아님" 철학과 동일하게 테스트도 하드 의존하지 않는다.
    #[test]
    fn build_record_ios_cert_reads_real_expiry_from_self_signed_test_cert() {
        let dir = temp_dir("cert-e2e");
        let cert_path = dir.join("test.pem");
        let key_path = dir.join("test.key");
        let spawn = Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-keyout",
                &key_path.to_string_lossy(),
                "-out",
                &cert_path.to_string_lossy(),
                "-days",
                "30",
                "-nodes",
                "-subj",
                "/CN=bildorak-signing-test",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let Ok(status) = spawn else {
            let _ = fs::remove_dir_all(&dir);
            return; // openssl 자체가 없는 환경 — 건너뛴다.
        };
        if !status.success() {
            let _ = fs::remove_dir_all(&dir);
            return;
        }

        let record = build_record(&cert_path).expect("등록 실패하면 안 된다");
        assert_eq!(record.kind, SigningKeyKind::IosCert);
        let expires_at = record.expires_at.expect("자체 서명 인증서는 만료일이 있어야 한다");
        let dt = chrono::DateTime::parse_from_rfc3339(&expires_at).expect("RFC3339 여야 한다");
        // -days 30 짜리라 지금부터 대략 30일 뒤여야 한다(실행 지연 감안 넉넉히 25~31일 범위로 확인).
        let days_left = (dt.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_days();
        assert!((25..=31).contains(&days_left), "예상 밖 만료일: {days_left}일 남음");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_signing_keys_missing_file_returns_empty() {
        let dir = temp_dir("load-empty");
        let keys = load_signing_keys(&dir).expect("파일 없음은 에러가 아니다");
        assert!(keys.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = temp_dir("roundtrip");
        let record = SigningKeyRecord {
            id: Uuid::new_v4().to_string(),
            kind: SigningKeyKind::AndroidKeystore,
            display_name: "release.jks".to_string(),
            file_path: "/tmp/release.jks".to_string(),
            expires_at: None,
            linked_project_ids: vec!["project-1".to_string()],
            android_signing: None,
            vault_path: None,
        };
        save_signing_keys(&dir, &[record.clone()]).expect("저장 실패하면 안 된다");
        let loaded = load_signing_keys(&dir).expect("읽기 실패하면 안 된다");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, record.id);
        assert_eq!(loaded[0].linked_project_ids, vec!["project-1".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    // ── keystore 안전 보관(볼트 복사) ────────────────────────────────────────────

    #[test]
    fn copy_keystore_into_vault_copies_bytes_and_leaves_original_untouched() {
        let source_dir = temp_dir("vault-source");
        let vault_dir = temp_dir("vault-dest");
        let source_path = source_dir.join("release.jks");
        let original_bytes = b"dummy keystore bytes for vault copy test";
        fs::write(&source_path, original_bytes).expect("원본 준비 실패하면 안 된다");

        let key_id = Uuid::new_v4().to_string();
        let dest = copy_keystore_into_vault(&vault_dir, &source_path, &key_id).expect("복사 실패하면 안 된다");

        // 파일명이 겹쳐도 안전하도록 key_id 접두사를 쓴다 — 볼트 경로 자체가 그 규칙을 따르는지 확인.
        assert_eq!(dest, vault_dir.join(format!("{key_id}-release.jks")));
        let copied_bytes = fs::read(&dest).expect("사본을 읽지 못했어요");
        assert_eq!(copied_bytes, original_bytes, "사본 내용이 원본과 바이트 단위로 같아야 한다");

        // 원본은 이동·삭제·수정되지 않아야 한다 — 여전히 원래 경로에 원래 내용 그대로 있어야 한다.
        assert!(source_path.is_file(), "원본이 그대로 남아 있어야 한다(이동 금지)");
        let original_after = fs::read(&source_path).expect("원본을 다시 읽지 못했어요");
        assert_eq!(original_after, original_bytes, "원본 내용이 바뀌면 안 된다");

        let _ = fs::remove_dir_all(&source_dir);
        let _ = fs::remove_dir_all(&vault_dir);
    }

    #[test]
    fn copy_keystore_into_vault_missing_source_is_error() {
        let vault_dir = temp_dir("vault-dest-missing-source");
        let missing_source = std::env::temp_dir().join(format!("bildorak-does-not-exist-{}.jks", Uuid::new_v4()));
        let result = copy_keystore_into_vault(&vault_dir, &missing_source, "some-key-id");
        assert!(result.is_err(), "존재하지 않는 원본은 에러여야 한다");
        let _ = fs::remove_dir_all(&vault_dir);
    }

    // ── keystore 안전 보관(클라우드 online-only 재시도) ──────────────────────────

    #[test]
    fn copy_with_retry_succeeds_after_transient_timeout_errors() {
        let mut attempts = 0;
        let result = copy_with_retry(&[0, 0, 0], || {
            attempts += 1;
            if attempts < 3 {
                Err(std::io::Error::from_raw_os_error(60)) // ETIMEDOUT(macOS) — 클라우드 다운로드 대기 재현.
            } else {
                Ok(42)
            }
        });
        assert_eq!(result.expect("세 번째 시도에서 성공해야 한다"), 42);
        assert_eq!(attempts, 3, "실패 2번 + 성공 1번 = 총 3번 시도해야 한다");
    }

    #[test]
    fn copy_with_retry_gives_up_after_exhausting_backoff_schedule() {
        let mut attempts = 0;
        let result = copy_with_retry(&[0, 0], || {
            attempts += 1;
            Err(std::io::Error::from_raw_os_error(60))
        });
        assert!(result.is_err(), "배열 소진 후에는 마지막 실패를 그대로 반환해야 한다");
        assert_eq!(attempts, 3, "최초 시도 1 + 재시도 2(delays 길이) = 3번 시도해야 한다");
    }

    #[test]
    fn copy_with_retry_does_not_retry_non_transient_errors() {
        let mut attempts = 0;
        let result = copy_with_retry(&[0, 0, 0], || {
            attempts += 1;
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        });
        assert!(result.is_err());
        assert_eq!(attempts, 1, "재시도 대상이 아닌 에러는 즉시 포기해야 한다(불필요한 대기 없음)");
    }

    #[test]
    fn copy_into_vault_with_retries_then_succeeds_and_renames_to_final_path() {
        let vault_dir = temp_dir("vault-retry-success");
        let tmp_dest = vault_dir.join("key-y-release.jks.part");
        let dest = vault_dir.join("key-y-release.jks");
        let bytes: &[u8] = b"real keystore bytes written on the successful retry";
        let mut attempts = 0;
        let attempt = || -> std::io::Result<u64> {
            attempts += 1;
            if attempts < 3 {
                Err(std::io::Error::from_raw_os_error(60))
            } else {
                fs::write(&tmp_dest, bytes).expect("임시본 쓰기 실패하면 안 된다");
                Ok(bytes.len() as u64)
            }
        };
        let result = copy_into_vault_with(
            &[0, 0, 0],
            attempt,
            Path::new("/dummy/source.jks"),
            &tmp_dest,
            &dest,
            Some(bytes.len() as u64),
        );
        let final_path = result.expect("세 번째 시도에서 성공하면 전체 결과도 성공해야 한다");
        assert_eq!(final_path, dest);
        assert!(!tmp_dest.exists(), "성공하면 임시본은 rename 되어 사라져야 한다");
        assert_eq!(fs::read(&dest).unwrap(), bytes, "최종본 내용이 성공 시점에 쓴 바이트와 같아야 한다");
        let _ = fs::remove_dir_all(&vault_dir);
    }

    #[test]
    fn copy_into_vault_with_removes_part_file_after_final_retry_failure() {
        let vault_dir = temp_dir("vault-cleanup-on-failure");
        let tmp_dest = vault_dir.join("key-x-release.jks.part");
        let dest = vault_dir.join("key-x-release.jks");
        // 재시도 도중 실제로 부분 바이트가 써졌다가(fs::copy 가 타임아웃 전에 일부를 이미 썼을 수 있는
        // 상황과 동일) 끝까지 실패하는 상황을 흉내낸다.
        let attempt = || -> std::io::Result<u64> {
            fs::write(&tmp_dest, b"partial download bytes").expect("더미 partial 쓰기 실패하면 안 된다");
            Err(std::io::Error::from_raw_os_error(60))
        };
        let result = copy_into_vault_with(&[0, 0], attempt, Path::new("/dummy/source.jks"), &tmp_dest, &dest, None);
        assert!(result.is_err(), "끝까지 실패하면 Err 이어야 한다");
        assert!(!tmp_dest.exists(), "재시도가 모두 실패하면 임시본(.part)은 정리돼야 한다");
        assert!(!dest.exists(), "최종본은 만들어지면 안 된다");
        let _ = fs::remove_dir_all(&vault_dir);
    }

    #[test]
    fn looks_like_cloud_path_detects_known_cloud_markers() {
        assert!(looks_like_cloud_path(Path::new("/Users/x/Library/CloudStorage/GoogleDrive-a/release.jks")));
        assert!(looks_like_cloud_path(Path::new(
            "/Users/x/Library/Mobile Documents/com~apple~CloudDocs/release.jks"
        )));
        assert!(looks_like_cloud_path(Path::new("/Users/x/Dropbox/release.jks")));
        assert!(looks_like_cloud_path(Path::new("/Users/x/OneDrive/release.jks")));
        assert!(!looks_like_cloud_path(Path::new("/Users/x/Documents/release.jks")));
    }

    #[test]
    fn copy_keystore_into_vault_error_mentions_cloud_storage_when_source_path_looks_cloud_backed() {
        let vault_dir = temp_dir("vault-cloud-hint");
        // 실제 클라우드 파일이 아니어도 된다 — 에러 메시지 보강은 "경로 문자열에 클라우드 마커가
        // 있는가"만 본다(looks_like_cloud_path). 존재하지 않는 경로라 fs::copy 는 재시도 없이(NotFound
        // 는 재시도 대상이 아니다) 바로 실패한다.
        let fake_cloud_source =
            std::env::temp_dir().join("Library").join("CloudStorage").join("GoogleDrive-test").join("release.jks");
        let result = copy_keystore_into_vault(&vault_dir, &fake_cloud_source, "cloud-key");
        let err = result.expect_err("존재하지 않는 원본은 에러여야 한다");
        assert!(err.contains("클라우드"), "클라우드 경로 힌트가 에러 메시지에 포함돼야 한다: {err}");
        let _ = fs::remove_dir_all(&vault_dir);
    }

    #[test]
    fn copy_keystore_into_vault_error_omits_cloud_hint_for_normal_local_path() {
        let vault_dir = temp_dir("vault-no-cloud-hint");
        let missing_source = std::env::temp_dir().join(format!("bildorak-does-not-exist-{}.jks", Uuid::new_v4()));
        let result = copy_keystore_into_vault(&vault_dir, &missing_source, "local-key");
        let err = result.expect_err("존재하지 않는 원본은 에러여야 한다");
        assert!(!err.contains("클라우드"), "일반 로컬 경로는 클라우드 힌트를 붙이면 안 된다: {err}");
        let _ = fs::remove_dir_all(&vault_dir);
    }

    // ── 클라우드 키 선제 알림(inspect_key_source) ─────────────────────────────────

    #[test]
    fn cloud_kind_label_matches_known_providers_and_falls_back_to_none() {
        assert_eq!(
            cloud_kind_label(Path::new("/Users/x/Library/Mobile Documents/com~apple~CloudDocs/release.jks")),
            Some("iCloud".to_string())
        );
        assert_eq!(
            cloud_kind_label(Path::new("/Users/x/Library/CloudStorage/GoogleDrive-a@b.com/release.jks")),
            Some("Google Drive".to_string())
        );
        assert_eq!(cloud_kind_label(Path::new("/Users/x/Dropbox/release.jks")), Some("Dropbox".to_string()));
        assert_eq!(
            cloud_kind_label(Path::new("/Users/x/Library/CloudStorage/OneDrive-Personal/release.jks")),
            Some("OneDrive".to_string())
        );
        // 이름 있는 4가지 라벨에 안 걸리는 CloudStorage 통합 제공자(예: Box)는 추측하지 않고 None.
        assert_eq!(cloud_kind_label(Path::new("/Users/x/Library/CloudStorage/Box-a/release.jks")), None);
        assert_eq!(cloud_kind_label(Path::new("/Users/x/Documents/release.jks")), None);
    }

    #[test]
    fn is_file_downloaded_true_for_normal_written_file() {
        let dir = temp_dir("downloaded-normal");
        let path = dir.join("release.jks");
        fs::write(&path, b"actual bytes written to disk").expect("쓰기 실패하면 안 된다");
        assert!(is_file_downloaded(&path), "실제 바이트를 쓴 파일은 다운로드된 것으로 판정돼야 한다");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_file_downloaded_true_when_metadata_unreadable() {
        let missing = std::env::temp_dir().join(format!("bildorak-inspect-missing-{}.jks", Uuid::new_v4()));
        assert!(is_file_downloaded(&missing), "메타데이터를 못 읽으면 보수적으로 다운로드됨으로 봐야 한다");
    }

    /// 완전 sparse 파일(set_len 만 하고 실제 데이터는 안 씀)로 클라우드 placeholder(len>0, blocks==0)를
    /// 흉내낸다. APFS 등 대부분의 파일시스템은 이런 "구멍(hole)"에 블록을 미리 할당하지 않지만, 이
    /// 가정이 성립하지 않는 드문 환경(파일시스템 차이)에서는 실제 클라우드 placeholder 를 흉내낼 수
    /// 없으므로 조용히 건너뛴다(openssl/keytool e2e 테스트와 동일한 "환경 의존은 하드 의존 아님" 철학).
    #[test]
    fn is_file_downloaded_false_for_sparse_placeholder_like_file() {
        let dir = temp_dir("downloaded-sparse");
        let path = dir.join("placeholder.jks");
        let file = fs::File::create(&path).expect("파일 생성 실패하면 안 된다");
        file.set_len(4096).expect("set_len 실패하면 안 된다");
        drop(file);

        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(&path).expect("메타데이터 읽기 실패하면 안 된다");
        if metadata.blocks() != 0 {
            let _ = fs::remove_dir_all(&dir);
            return; // 이 환경은 sparse 파일도 블록을 할당한다 — placeholder 를 재현할 수 없어 건너뛴다.
        }
        assert!(!is_file_downloaded(&path), "len>0 && blocks==0 이면 다운로드 안 된 것으로 판정해야 한다");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspect_key_source_missing_file_is_error() {
        let dir = temp_dir("inspect-missing");
        let result = inspect_key_source(&dir.join("nope.jks"));
        assert!(result.is_err(), "존재하지 않는 파일은 에러여야 한다");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspect_key_source_detects_cloud_path_and_folder_name() {
        let dir = temp_dir("inspect-cloud");
        let dropbox_dir = dir.join("Dropbox").join("하루블록키");
        fs::create_dir_all(&dropbox_dir).unwrap();
        let path = dropbox_dir.join("release.jks");
        fs::write(&path, b"dummy keystore bytes for test").unwrap();

        let info = inspect_key_source(&path).expect("등록 실패하면 안 된다");
        assert!(info.is_cloud, "Dropbox 경로는 클라우드로 판정돼야 한다");
        assert_eq!(info.folder_name, "하루블록키");
        assert_eq!(info.cloud_kind.as_deref(), Some("Dropbox"));
        assert!(info.is_downloaded, "실제로 바이트를 쓴 파일은 다운로드된 것으로 판정돼야 한다");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspect_key_source_non_cloud_path_has_no_cloud_kind() {
        let dir = temp_dir("inspect-local");
        let path = dir.join("release.jks");
        fs::write(&path, b"dummy keystore bytes for test").unwrap();

        let info = inspect_key_source(&path).expect("등록 실패하면 안 된다");
        assert!(!info.is_cloud, "일반 로컬 경로는 클라우드가 아니어야 한다");
        assert!(info.cloud_kind.is_none(), "비클라우드는 cloud_kind 가 없어야 한다");
        assert!(info.is_downloaded);
        let _ = fs::remove_dir_all(&dir);
    }

    // ── Android release 서명 자동 주입(다음 단계) ────────────────────────────────

    #[test]
    fn extract_sha256_fingerprint_reads_real_keytool_output_format() {
        // 실측 캡처(keytool, JDK 17, 2026-08-17): `keytool -printcert -jarfile`/`keytool -list -v`
        // 양쪽 다 이 형태로 "SHA256:" 줄을 낸다(주변 문구는 로케일에 따라 달라도 이 줄 자체는 상수).
        let raw = "소유자: CN=Release A, O=Test\n\
                    발행자: CN=Release A, O=Test\n\
                    인증서 지문:\n\
                    \t SHA1: 6E:01:80:DB:41:D2:13:FA:0C:08:7E:08:C5:0E:D6:6C:EB:EE:7D:25\n\
                    \t SHA256: 45:83:87:68:00:13:51:DE:7F:7E:FD:DE:D9:41:F3:4C:3E:16:37:F8:DC:15:8C:00:F7:6F:53:D4:89:EA:25:21\n\
                    서명 알고리즘 이름: SHA256withRSA\n";
        let fingerprint = extract_sha256_fingerprint(raw).expect("실측 포맷을 파싱하지 못했어요");
        assert_eq!(
            fingerprint,
            "45:83:87:68:00:13:51:DE:7F:7E:FD:DE:D9:41:F3:4C:3E:16:37:F8:DC:15:8C:00:F7:6F:53:D4:89:EA:25:21"
        );
    }

    #[test]
    fn extract_sha256_fingerprint_none_when_unsigned() {
        // 실측 캡처: 서명 안 된 zip 에 `keytool -printcert -jarfile` 를 돌리면 이 한 줄만 나오고 종료
        // 코드는 0 이다 — 그래서 종료 코드가 아니라 이 함수(SHA256 줄 존재 여부)로 판정해야 한다.
        assert_eq!(extract_sha256_fingerprint("서명된 jar 파일이 아닙니다.\n"), None);
        assert_eq!(extract_sha256_fingerprint(""), None);
    }

    #[test]
    fn parse_keystore_validity_end_reads_real_keytool_output_format() {
        // 실측 캡처(keytool, JDK 17, 2026-08-19, 이 머신 한국어 로케일): "적합한 시작 날짜: <A> 종료
        // 날짜: <B>" 한 줄에 Java Date::toString() 값 두 개가 나온다(영문 JDK 는 "Valid from: <A> until:
        // <B>" 로 라벨만 다르다 — extract_keytool_java_dates 문서 참고).
        let raw = "별칭 이름: testalias\n\
                    생성 날짜: 2026. 8. 19.\n\
                    항목 유형: PrivateKeyEntry\n\
                    인증서 체인 길이: 1\n\
                    인증서[1]:\n\
                    소유자: CN=bildorak-test\n\
                    발행자: CN=bildorak-test\n\
                    일련 번호: e4e64fc707e91aff\n\
                    적합한 시작 날짜: Wed Aug 19 01:43:17 KST 2026 종료 날짜: Sat Aug 16 01:43:17 KST 2036\n\
                    인증서 지문:\n\
                    \t SHA256: 3E:CE:E8:91:A5:92:FA:78:BB:77:4E:D4:24:1B:90:C2:D5:02:04:27:7F:03:03:AB:28:AD:F8:61:EF:A6:98:B9\n";
        let expiry = parse_keystore_validity_end(raw).expect("실측 포맷을 파싱하지 못했어요");
        let parsed = chrono::DateTime::parse_from_rfc3339(&expiry).expect("RFC3339 여야 한다");

        // 이 함수는 시간대 약어(KST) 문구를 신뢰하지 않고 "이 테스트를 지금 돌리는 머신의 로컬 시간대"
        // 로 해석한다(extract_keytool_java_dates 문서 참고) — 그래서 기대값도 chrono::Local 로 똑같이
        // 계산해 비교한다. 이렇게 하면 이 테스트가 어느 시간대 머신에서 실행되어도 항상 유효하다.
        use chrono::TimeZone;
        let naive = chrono::NaiveDateTime::parse_from_str("Aug 16 01:43:17 2036", "%b %d %H:%M:%S %Y")
            .expect("고정 포맷 파싱 실패하면 안 된다");
        let expected = chrono::Local.from_local_datetime(&naive).single().expect("존재하는 로컬 시각이어야 한다");
        assert_eq!(parsed.with_timezone(&chrono::Utc), expected.with_timezone(&chrono::Utc));
    }

    #[test]
    fn parse_keystore_validity_end_none_when_unparseable() {
        assert_eq!(parse_keystore_validity_end("아무 날짜도 없는 텍스트\n"), None);
        assert_eq!(parse_keystore_validity_end(""), None);
        // 날짜 패턴이 딱 1개만 있으면(종료일이 없음) 인덱스 1 이 없어 None 이어야 한다.
        assert_eq!(parse_keystore_validity_end("시작: Wed Aug 19 01:43:17 KST 2026 뿐\n"), None);
    }

    #[test]
    fn keychain_store_read_delete_round_trip() {
        // 실제 macOS 로그인 키체인에 대고 검증한다(실측: add-generic-password -U 로 만든 항목은
        // find-generic-password -w 로 프롬프트 없이 바로 읽힌다). 매 실행 유일한 service 이름을 써서
        // 다른 테스트/실행과 충돌하지 않게 하고, 끝나면 반드시 지운다.
        let service = format!("bildorak-test-keychain-{}", Uuid::new_v4());
        let account = "test-account";
        let password = "s3cr3t-pw-!@#";

        store_keychain_password(&service, account, password).expect("저장 실패하면 안 된다");
        let read_back = read_keychain_password(&service, account).expect("조회 실패하면 안 된다");
        assert_eq!(read_back, password, "trailing newline 없이 원문 그대로 읽혀야 한다");

        delete_keychain_password(&service, account);
        let after_delete = read_keychain_password(&service, account);
        assert!(after_delete.is_err(), "삭제 후에는 조회가 실패해야 한다");
    }

    #[test]
    fn register_android_signing_rejects_empty_alias_or_password() {
        // 검증(alias/비밀번호 빈 값)이 keystore_path 를 실제로 열어보기 전에 먼저 걸리므로 존재하지
        // 않는 더미 경로로도 충분하다.
        let dummy = Path::new("/tmp/bildorak-test-does-not-exist.jks");
        assert!(register_android_signing(dummy, "key-1", "", "pw", "pw").is_err());
        assert!(register_android_signing(dummy, "key-1", "  ", "pw", "pw").is_err());
        assert!(register_android_signing(dummy, "key-1", "alias", "", "pw").is_err());
        assert!(register_android_signing(dummy, "key-1", "alias", "pw", "").is_err());
    }

    #[test]
    fn register_android_signing_stores_both_passwords_in_keychain() {
        // 이 테스트는 keychain 저장/조회만 검증한다 — 실제 keystore 파일이 없어도 등록(비밀번호 저장)
        // 자체는 성공해야 한다(cert 메타데이터 추출은 best-effort 로 실패해도 조용히 None). cert 메타
        // 추출 e2e 는 register_android_signing_extracts_cert_metadata_when_keystore_exists 가 따로 본다.
        let dummy = Path::new("/tmp/bildorak-test-does-not-exist.jks");
        let key_id = format!("test-key-{}", Uuid::new_v4());
        let config = register_android_signing(dummy, &key_id, "release-alias", "storepw-1", "keypw-2")
            .expect("등록 실패하면 안 된다");
        assert_eq!(config.key_alias, "release-alias");
        assert_eq!(config.keychain_account, "release-alias");
        assert!(config.cert_expiry.is_none(), "존재하지 않는 keystore 는 만료일을 못 뽑아야 한다");
        assert!(config.cert_sha256.is_none(), "존재하지 않는 keystore 는 지문을 못 뽑아야 한다");
        assert_eq!(
            read_keychain_password(&config.store_password_service, &config.keychain_account).as_deref(),
            Ok("storepw-1")
        );
        assert_eq!(
            read_keychain_password(&config.key_password_service, &config.keychain_account).as_deref(),
            Ok("keypw-2")
        );
        forget_android_signing_secrets(&config);
        assert!(read_keychain_password(&config.store_password_service, &config.keychain_account).is_err());
        assert!(read_keychain_password(&config.key_password_service, &config.keychain_account).is_err());
    }

    /// cert 메타데이터(만료일 + SHA-256 지문) 등록 e2e — 이 머신에서 자체 생성한 테스트 keystore
    /// (keytool -genkeypair)로 register_android_signing 이 실제로 두 값을 채우는지 확인한다(실키·실제
    /// 비밀번호 절대 사용 안 함, 보안 원칙). keytool 이 없으면(드묾) 조용히 건너뛴다(다른
    /// keytool e2e 테스트와 동일한 "도구 없음은 하드 의존 아님" 철학).
    #[test]
    fn register_android_signing_extracts_cert_metadata_when_keystore_exists() {
        let Some(keytool) = child_env::resolve_jdk_tool("keytool") else {
            return; // JDK 없는 환경 — 건너뛴다.
        };
        let dir = temp_dir("cert-metadata-e2e");
        let keystore_path = dir.join("release.jks");
        let alias = format!("alias-{}", Uuid::new_v4());
        let store_pw = "storepw-meta-test";
        let status = Command::new(&keytool)
            .args(["-genkeypair", "-storetype", "JKS", "-keystore"])
            .arg(&keystore_path)
            .args(["-storepass", store_pw, "-keypass", store_pw, "-alias", &alias])
            .args(["-dname", "CN=bildorak-cert-meta-test", "-validity", "3650", "-keyalg", "RSA", "-keysize", "2048"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("keytool 실행 자체가 실패하면 안 된다");
        if !status.success() || !keystore_path.is_file() {
            let _ = fs::remove_dir_all(&dir);
            return; // 이 머신에서 keytool 생성이 안 되는 드문 환경 — 건너뛴다.
        }

        let key_id = format!("test-key-{}", Uuid::new_v4());
        let config = register_android_signing(&keystore_path, &key_id, &alias, store_pw, store_pw)
            .expect("등록 실패하면 안 된다");

        let sha256 = config.cert_sha256.clone().expect("자체생성 keystore 는 SHA-256 지문을 얻어야 한다");
        assert!(sha256.contains(':'), "지문은 콜론 구분 16진수 형태여야 한다: {sha256}");

        let expiry = config.cert_expiry.clone().expect("자체생성 keystore 는 만료일을 얻어야 한다");
        let dt = chrono::DateTime::parse_from_rfc3339(&expiry).expect("RFC3339 여야 한다");
        // -validity 3650(약 10년)짜리라 지금부터 대략 10년 후여야 한다(실행 지연 감안 넉넉히 3600~3660일
        // 범위로 확인).
        let days_left = (dt.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_days();
        assert!((3600..=3660).contains(&days_left), "예상 밖 만료일: {days_left}일 남음");

        forget_android_signing_secrets(&config);
        let _ = fs::remove_dir_all(&dir);
    }

    /// keytool/jarsigner e2e — 이 머신에서 자체 생성한 테스트 keystore 2개(A/B, keytool -genkeypair)로
    /// 실제 jarsigner 서명 + verify_release_signing 전체 경로를 검증한다(실키·실제 비밀번호 절대 사용
    /// 안 함, 보안 원칙). keytool/jarsigner/zip 이 이 머신에 없으면(드묾) 조용히 건너뛴다
    /// (build_record_ios_cert_reads_real_expiry_from_self_signed_test_cert 와 동일한 "도구 없음은 하드
    /// 의존 아님" 철학).
    #[test]
    fn verify_release_signing_matches_correct_keystore_and_rejects_wrong_one() {
        let Some(keytool) = child_env::resolve_jdk_tool("keytool") else {
            return; // JDK 없는 환경 — 건너뛴다.
        };
        let dir = temp_dir("jarsigner-e2e");

        let gen_keystore = |name: &str, alias: &str, store_pw: &str| -> PathBuf {
            let path = dir.join(name);
            Command::new(&keytool)
                .args([
                    "-genkeypair",
                    "-storetype",
                    "JKS",
                    "-keystore",
                ])
                .arg(&path)
                .args(["-storepass", store_pw, "-keypass", store_pw, "-alias", alias])
                .args(["-dname", "CN=bildorak-test", "-validity", "1", "-keyalg", "RSA", "-keysize", "2048"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("keytool 실행 자체가 실패하면 안 된다");
            path
        };
        let keystore_a = gen_keystore("a.jks", "aliasA", "storepwA");
        let keystore_b = gen_keystore("b.jks", "aliasB", "storepwB");
        if !keystore_a.is_file() || !keystore_b.is_file() {
            let _ = fs::remove_dir_all(&dir);
            return; // keytool 은 있지만 생성 자체가 실패한 드문 환경 — 건너뛴다.
        }

        // 더미 aab(그냥 zip) 준비 — jarsigner 는 zip 컨테이너면 서명 대상으로 받아들인다(실측 확인).
        let content_dir = dir.join("contents");
        fs::create_dir_all(&content_dir).unwrap();
        fs::write(content_dir.join("base.txt"), b"hello").unwrap();
        let artifact = dir.join("app-release.aab");
        let zip_status = Command::new("zip")
            .args(["-r", &artifact.to_string_lossy(), "."])
            .current_dir(&content_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let Ok(zip_status) = zip_status else {
            let _ = fs::remove_dir_all(&dir);
            return; // zip 명령이 없는 드문 환경 — 건너뛴다.
        };
        if !zip_status.success() {
            let _ = fs::remove_dir_all(&dir);
            return;
        }

        let Some(jarsigner) = child_env::resolve_jdk_tool("jarsigner") else {
            let _ = fs::remove_dir_all(&dir);
            return;
        };
        let sign_status = Command::new(&jarsigner)
            .args(["-keystore"])
            .arg(&keystore_a)
            .args(["-storepass", "storepwA", "-keypass", "storepwA"])
            .arg(&artifact)
            .arg("aliasA")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("jarsigner 실행 자체가 실패하면 안 된다");
        assert!(sign_status.success(), "테스트 keystore A 로 서명이 성공해야 한다");

        // keystore A(실제로 서명한 키) 로는 일치해야 한다.
        verify_release_signing(&artifact, &keystore_a, "aliasA", "storepwA")
            .expect("서명한 keystore 로 검증하면 통과해야 한다");

        // keystore B(다른 키, "debug 키로 잘못 서명됨" 시나리오)로는 불일치해야 한다.
        let mismatch = verify_release_signing(&artifact, &keystore_b, "aliasB", "storepwB");
        assert!(mismatch.is_err(), "다른 keystore 와는 지문이 달라 실패해야 한다");

        let _ = fs::remove_dir_all(&dir);
    }

    // ── iOS 배포 인증서 Team ID 조회(parse_distribution_team_id) ───────────────────────────────
    // 값은 이 머신의 실제 인증서가 아니라 형식만 같은 예시 이름/ID 로 둔다 — team_id
    // 자체는 비밀이 아니지만(방침 배경) 실제 인증서 소유자 이름을 소스에 남길 이유는 없다.

    #[test]
    fn parse_distribution_team_id_extracts_from_real_output_format() {
        // `security find-identity -v -p codesigning` 실측 포맷 그대로(줄 번호/해시/따옴표 구조),
        // 이름/ID 값만 예시로 대체했다.
        let raw = "  1) AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA \"Apple Distribution: Sample Developer (ABCD123456)\"\n  2) BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB \"Apple Development: Sample Developer (WXYZ987654)\"\n     2 valid identities found\n";
        assert_eq!(parse_distribution_team_id(raw), Some("ABCD123456".to_string()));
    }

    #[test]
    fn parse_distribution_team_id_ignores_development_only_identities() {
        let raw = "  1) BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB \"Apple Development: Sample Developer (WXYZ987654)\"\n     1 valid identities found\n";
        assert_eq!(parse_distribution_team_id(raw), None);
    }

    #[test]
    fn parse_distribution_team_id_none_when_no_identities_found() {
        assert_eq!(parse_distribution_team_id("     0 valid identities found\n"), None);
        assert_eq!(parse_distribution_team_id(""), None);
    }
}
