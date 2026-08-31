// releases.rs - 릴리스 기록 저장/조회(릴리스 관리 1차 슬라이스). 등록된 앱별로 "언제 어떤 버전을
// 어느 채널에 냈고 지금 어떤 상태인지"를 간단히 남긴다. 빌드 이력 연결·GitHub 연동·제출 자동화는
// 범위 밖(다음 로드맵 단계) - 지금은 순수 수동 기록이다.
//
// 저장은 store.rs 의 write_json_atomic(temp+rename)을 재사용한다(signing.rs 와 동일 패턴). projects.json/
// signing_keys.json 과 마찬가지로 목록이 작은 개인 데스크톱 앱이라 등록/수정/삭제 모두 배열 전체를
// 다시 쓰는 단순 read-modify-write 로 충분하다(전용 파일 락 없음 - store.rs 주석과 동일 이유).

use crate::model::ReleaseRecord;
use crate::store::write_json_atomic;
use std::path::{Path, PathBuf};

const RELEASES_FILE: &str = "releases.json";

fn releases_file_path(base_dir: &Path) -> PathBuf {
    base_dir.join(RELEASES_FILE)
}

/// 저장된 릴리스 목록을 읽는다. 파일이 없으면(첫 등록 전) 빈 목록 - store.rs::load_projects_from_dir
/// 와 동일 규칙.
pub fn load_releases_from_dir(base_dir: &Path) -> Result<Vec<ReleaseRecord>, String> {
    let path = releases_file_path(base_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("등록된 릴리스 목록을 읽지 못했어요: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&raw).map_err(|e| format!("등록된 릴리스 목록이 손상됐어요: {e}"))
}

/// 목록 전체를 저장한다(pretty JSON) - store.rs::save_projects_to_dir 와 동일 규칙.
pub fn save_releases_to_dir(base_dir: &Path, releases: &[ReleaseRecord]) -> Result<(), String> {
    let path = releases_file_path(base_dir);
    let raw = serde_json::to_string_pretty(releases)
        .map_err(|e| format!("저장할 데이터를 만들지 못했어요: {e}"))?;
    write_json_atomic(&path, &raw).map_err(|e| format!("릴리스 목록을 저장하지 못했어요: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ReleaseChannel, ReleaseStatus};
    use std::fs;

    fn temp_base_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bildorak-releases-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("failed to create temp base dir");
        dir
    }

    fn fake_release() -> ReleaseRecord {
        ReleaseRecord {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: "test-project".to_string(),
            version: "1.0.0".to_string(),
            build_number: Some("1".to_string()),
            channel: ReleaseChannel::AppStore,
            status: ReleaseStatus::Preparing,
            notes: String::new(),
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            updated_at: "2026-01-01T00:00:00+00:00".to_string(),
            released_at: None,
        }
    }

    /// store.rs::load_projects_from_dir_round_trips_with_save 와 동일한 왕복 검증 - 파일이 없으면 빈
    /// 목록, 저장 후 다시 읽으면 그대로 돌아와야 한다.
    #[test]
    fn load_releases_from_dir_round_trips_with_save() {
        let dir = temp_base_dir();
        assert!(load_releases_from_dir(&dir).expect("빈 목록이어야 한다").is_empty());

        let release = fake_release();
        save_releases_to_dir(&dir, std::slice::from_ref(&release)).expect("저장 실패하면 안 된다");

        let loaded = load_releases_from_dir(&dir).expect("읽기 실패하면 안 된다");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, release.id);
        assert_eq!(loaded[0].version, release.version);
        assert_eq!(loaded[0].channel, release.channel);
        assert_eq!(loaded[0].status, release.status);

        let _ = fs::remove_dir_all(&dir);
    }
}
