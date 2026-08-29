// pubspec.rs — 선택한 폴더에서 Flutter 프로젝트(pubspec.yaml)를 찾고 이름/버전/플랫폼을 읽는다.
// 규칙(1단계 스펙): 선택 폴더 직속에 pubspec.yaml 이 있으면 그 폴더, 없으면
// "<선택 폴더>/app/pubspec.yaml" 을 한 단계만 더 본다(실제 Flutter 프로젝트 2개에서 이 패턴 —
// /Users/you/projects/myapp/app/pubspec.yaml, /Users/you/projects/otherapp/app/pubspec.yaml 로 실측 확인함).
// 그 외 깊이는 보지 않는다 — 못 찾으면 비개발자 문구로 안내(추측하지 않는다).

use crate::model::Platform;
use std::path::{Path, PathBuf};

pub struct DetectedProject {
    pub repo_path: PathBuf,
    pub name: String,
    pub version: Option<String>,
    pub build_number: Option<String>,
    pub platforms: Vec<Platform>,
}

/// pubspec.yaml 원문에서 "name: xxx" 한 줄만 뽑는다. 못 찾으면 None.
fn parse_name(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("name:") {
            let name = rest.trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// "version: 1.1.6+11" → ("1.1.6", Some("11")). 검증된 파싱 규칙 그대로 따른다.
fn parse_version(content: &str) -> Option<(String, Option<String>)> {
    for line in content.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("version:") {
            let raw = rest.trim();
            if raw.is_empty() {
                continue;
            }
            return Some(match raw.split_once('+') {
                Some((version, build)) => (version.to_string(), Some(build.to_string())),
                None => (raw.to_string(), None),
            });
        }
    }
    None
}

fn detect_platforms(repo_path: &Path) -> Vec<Platform> {
    let mut platforms = Vec::new();
    if repo_path.join("ios").is_dir() {
        platforms.push(Platform::Ios);
    }
    if repo_path.join("android").is_dir() {
        platforms.push(Platform::Android);
    }
    platforms
}

/// 선택 폴더에서 pubspec.yaml 을 찾는다 — 실제 프로젝트 루트 경로만 반환(직속 또는 app/ 하위 1단계).
fn find_pubspec_root(selected: &Path) -> Option<PathBuf> {
    if selected.join("pubspec.yaml").is_file() {
        return Some(selected.to_path_buf());
    }
    let nested = selected.join("app");
    if nested.join("pubspec.yaml").is_file() {
        return Some(nested);
    }
    None
}

/// 사용자가 고른 폴더 → 감지 결과. 못 찾으면 비개발자 문구의 에러를 그대로 반환(호출부가 그대로 표시).
pub fn detect_project(selected: &Path) -> Result<DetectedProject, String> {
    let found = find_pubspec_root(selected).ok_or_else(|| {
        "이 폴더에서 Flutter 프로젝트를 찾지 못했어요. pubspec.yaml 파일이 있는 폴더나, \
         그 폴더를 담고 있는 폴더를 선택해 주세요."
            .to_string()
    })?;

    // repo_path 는 2차(빌드 실행)부터 자식 프로세스의 cwd 로 그대로 쓰인다 — symlink 로 실제 위치가
    // 가려진 경로를 실행 시점까지 들고 가지 않도록 여기서 실제 경로로 확정하고, 감지~등록 사이 폴더가
    // 사라졌을 가능성까지 다시 확인한다(설계 요구사항).
    let repo_path = std::fs::canonicalize(&found)
        .map_err(|_| "선택한 폴더 경로를 확인하지 못했어요.".to_string())?;
    if !repo_path.is_dir() {
        return Err("선택한 폴더를 찾을 수 없어요.".to_string());
    }

    let content = std::fs::read_to_string(repo_path.join("pubspec.yaml"))
        .map_err(|_| "pubspec.yaml 파일을 읽지 못했어요.".to_string())?;

    let name =
        parse_name(&content).ok_or_else(|| "pubspec.yaml 에서 앱 이름을 찾지 못했어요.".to_string())?;
    let (version, build_number) = match parse_version(&content) {
        Some((v, b)) => (Some(v), b),
        None => (None, None),
    };
    let platforms = detect_platforms(&repo_path);

    Ok(DetectedProject {
        repo_path,
        name,
        version,
        build_number,
        platforms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_name_and_version_with_build_number() {
        let content = "name: myapp\ndescription: test\nversion: 1.1.6+11\n";
        assert_eq!(parse_name(content), Some("myapp".to_string()));
        assert_eq!(
            parse_version(content),
            Some(("1.1.6".to_string(), Some("11".to_string())))
        );
    }

    #[test]
    fn parses_version_without_build_number() {
        let content = "name: sample\nversion: 2.0.0\n";
        assert_eq!(
            parse_version(content),
            Some(("2.0.0".to_string(), None))
        );
    }

    #[test]
    fn missing_fields_return_none() {
        let content = "description: no name or version here\n";
        assert_eq!(parse_name(content), None);
        assert_eq!(parse_version(content), None);
    }
}
