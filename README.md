# bildorak (빌도락)

**[한국어](README.md)** | [English](README.en.md)

**앱을 만들고, 서명하고, 스토어에 올릴 준비까지 - 내 컴퓨터에서, 간단한 데스크톱 앱으로.**

빌도락은 Flutter 앱을 빌드하고, 서명 키를 관리하고, 스토어 제출용 빌드(안드로이드 `.aab` /
iOS `.ipa`)를 로컬에서 만들어주는 데스크톱 GUI입니다. CI 서비스 없이, 커맨드라인 서명 씨름
없이 앱을 스토어까지 가져가고 싶은 사람을 위한 도구예요.

무료 오픈소스(MIT). Tauri 2 + React + Rust로 만들었습니다.

> ⚠️ 초기 / 포트폴리오 단계 프로젝트입니다. 현재는 macOS 중심이에요 (iOS 빌드는 Xcode 필요).

## 주요 기능

- **로컬 빌드** - 안드로이드 디버그(`apk`)·릴리스(`aab`), iOS 시뮬레이터·릴리스(`ipa`, App Store export)
- **서명 간편화**
  - 서명 키(`.jks`)·애플 `.p8` 키를 컴퓨터에서 자동 탐색
  - keystore 비밀번호는 macOS 키체인에만 저장 (파일·로그엔 절대 안 남김)
  - 프로젝트 `key.properties`에서 비밀번호 자동 채움 - 직접 입력할 필요 없어요
  - 클라우드(구글 드라이브·iCloud 등)에 있는 키도 인식하고, 다운로드가 필요하면 미리 알려줌
  - keystore를 앱 금고에 안전 보관 (원본은 그대로, 복사만)
  - 인증서 만료일·지문 한눈에
- **앱별 체크리스트** - 앱마다 서명/업로드 준비 상태를 한눈에
- **스토어 제출용 빌드** - 안드로이드 `.aab`는 Play Console, iOS `.ipa`는 Transporter로 수동 업로드
- **빌드 히스토리 + 완료 알림**, 다크 모드, 한국어/English 지원

## CLI - AI 에이전트·자동화용

GUI와 같은 엔진·같은 데이터를 쓰는 커맨드라인 도구 `bildorak-cli`가 함께 들어 있어요.
Claude Code 같은 AI 코딩 에이전트나 CI 스크립트가 터미널에서 빌도락을 그대로 쓸 수 있습니다.

```bash
bildorak-cli apps                                # 등록된 앱 목록
bildorak-cli build <앱이름> --target ios-release  # 서명된 스토어용 빌드
bildorak-cli status <앱이름>                      # 출시 준비 체크리스트
bildorak-cli keys                                # 서명 키 목록 (비밀번호는 절대 출력 안 함)
bildorak-cli doctor                              # 환경 점검 (Flutter/Xcode/Android SDK)
```

- 모든 명령에 `--json` - 구조화된 출력으로 프로그램이 결과를 파싱할 수 있어요
- 종료 코드 규약: 성공 `0` / 실패 비`0` - 스크립트·CI에서 그대로 판단
- 사람은 GUI로 서명 키를 한 번 등록하고, AI는 CLI로 빌드를 반복하는 분업을 의도했습니다

## 요구 사항

- macOS (iOS 빌드는 Xcode 필요), [Flutter](https://flutter.dev) 설치
- 소스에서 직접 빌드하려면 Rust + Node.js

## 소스에서 빌드

```bash
npm install
npm run tauri dev      # 개발 모드 실행
npm run tauri build    # 배포용 앱 생성
cargo build --release --manifest-path src-tauri/Cargo.toml   # CLI: src-tauri/target/release/bildorak-cli
```

## 안전

서명 키·비밀번호는 어디에도 업로드되지 않아요. 전부 로컬에서 처리됩니다.

- 비밀번호는 macOS 키체인에만.
- keystore는 **복사**(이동 아님)해 앱 금고에 보관 - 원본은 그대로.
- iOS 서명은 기존 Xcode 인증서를 그대로 사용.

## 라이선스

MIT © 2026 Gradibo. [LICENSE](LICENSE) 참고.

만든 곳: [Gradibo](https://github.com/gradibo) - 1인 메이커 스튜디오. 출시 전 자체 앱들로
실전 테스트했습니다.

## 기여

이슈·PR 환영합니다. 재미와 커뮤니티를 위해 만든 작은 프로젝트예요.
