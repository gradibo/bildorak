# 빌도락 릴리스 가이드 (자동 업데이트)

> **누가 언제 읽는가**: 빌도락 새 버전을 GitHub Releases 에 올려 기존 사용자에게 자동 업데이트로
> 전달할 때 읽는다. 이 문서는 공개 가능한 절차만 다룬다 — 실제 서명 개인키/패스프레이즈는 여기 없다.

## 1. 개요

빌도락은 Tauri 공식 [updater 플러그인](https://v2.tauri.app/plugin/updater/)으로 자동 업데이트를
한다. 앱이 시작할 때(설정에서 켜져 있으면) 아래 URL 의 `latest.json` 을 조용히 확인하고, 더 높은
버전이 있으면 모달로 안내한 뒤 원클릭으로 내려받아 설치·재시작한다.

```
https://github.com/gradibo/bildorak/releases/latest/download/latest.json
```

이 URL 은 GitHub Releases 의 "최신 릴리스"에 `latest.json` 이라는 이름의 asset 이 붙어 있어야 응답한다.
아직 릴리스를 하나도 안 올렸거나 asset 이 없으면 404 가 오는데, 앱은 이 경우 사용자에게 아무것도
보여주지 않고 콘솔에만 로그를 남긴다(방해 금지 원칙, `src/components/UpdateModal.tsx` 참고).

## 2. 서명 키

Tauri updater 는 업데이트 파일이 우리가 만든 게 맞는지 minisign 서명으로 검증한다. 키는 최초 1회만
만들면 되고, 버전마다 새로 만들지 않는다.

- **개인키**: `~/.tauri/bildorak-updater.key` — **레포 밖**에만 있다. 절대 커밋하지 않는다
  (`.gitignore` 의 `*.key` 패턴이 실수로 레포 안에 들어와도 걸러준다). 파일 권한은 `600`
  (`chmod 600 ~/.tauri/bildorak-updater.key`)으로 소유자만 읽게 해 둔다.
- **공개키**: `~/.tauri/bildorak-updater.key.pub` — `src-tauri/tauri.conf.json` 의
  `plugins.updater.pubkey` 에 이미 반영돼 있다(공개 정보라 커밋해도 안전).
- 패스프레이즈 없이 생성했다(`-p ""`) — 개인키 파일 자체가 유일한 보호막이므로 파일 접근 권한(600,
  레포 밖 홈 디렉터리)이 실질적인 보안 경계다.
- 개인키/패스프레이즈를 잃어버리면 기존 사용자에게 새 서명으로 업데이트를 발급할 수 없다(구버전 앱은
  옛 공개키만 신뢰한다) — `~/.tauri/` 는 이 머신의 일반 백업 대상에 포함되어 있는지 확인해 둔다.

새로 키를 만들어야 한다면(분실 시 재발급 등):

```bash
mkdir -p ~/.tauri
npx tauri signer generate --ci --password "" -w ~/.tauri/bildorak-updater.key
chmod 600 ~/.tauri/bildorak-updater.key
```

재발급하면 `~/.tauri/bildorak-updater.key.pub` 내용을 `tauri.conf.json` 의
`plugins.updater.pubkey` 에도 다시 반영해야 한다 — 안 하면 새 서명을 옛 공개키가 거부한다.

## 3. 서명해서 빌드하기

빌드 시점에 개인키를 환경변수로 넘기면 `tauri build` 가 `.app.tar.gz` 옆에 `.sig` 서명 파일을 같이
만든다(`tauri.conf.json` 의 `bundle.createUpdaterArtifacts: true` 가 이 산출물을 켠다).

```bash
cd bildorak
TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/bildorak-updater.key) \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
npm run tauri build
```

⚠️ **`TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""` 를 꼭 같이 넘겨야 한다** — 이 변수를 아예 안 주면
패스프레이즈 없는 키인데도 Tauri CLI 가 "Decrypting updater signing key, expect a prompt for
password"라며 터미널에서 비밀번호를 대화형으로 물어보려 하고, TTY 가 없는 환경(에이전트/CI 등)에서는
`failed to decode secret key: incorrect updater private key password: Device not configured (os
error 6)` 로 실패한다(2026-08-31 실측 확인). 빈 문자열을 명시하면 프롬프트 없이 곧장 진행된다.

개인키를 파일 그대로 넘기고 싶으면(내용을 셸 히스토리에 남기지 않는 방법) 아래처럼 경로 변수를 써도
된다(이때도 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""` 는 그대로 필요하다):

```bash
TAURI_SIGNING_PRIVATE_KEY_PATH=~/.tauri/bildorak-updater.key \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
npm run tauri build
```

빌드가 끝나면 (macOS, Apple Silicon 기준) 아래 산출물이 생긴다:

```
src-tauri/target/release/bundle/macos/빌도락.app          # 앱 번들
src-tauri/target/release/bundle/macos/빌도락.app.tar.gz    # 업데이터 아카이브
src-tauri/target/release/bundle/macos/빌도락.app.tar.gz.sig # 위 파일 서명(공개해도 되는 값)
src-tauri/target/release/bundle/dmg/*.dmg                  # 신규 설치용 배포 이미지
```

## 4. 업로드용 ASCII 이름으로 복사 (필수 - 건너뛰면 업데이트 404)

GitHub 은 release asset 의 비ASCII 파일명(한글 등)을 업로드 시 **강제로 개명**한다.
그대로 올리면 latest.json 의 URL 과 실제 asset 이름이 어긋나 사용자의 "지금 업데이트"가
404 로 실패한다. 반드시 업로드 전에 ASCII 이름으로 복사한다:

```bash
cd src-tauri/target/release/bundle
mv "macos/빌도락.app.tar.gz"      "macos/bildorak.app.tar.gz"
mv "macos/빌도락.app.tar.gz.sig"  "macos/bildorak.app.tar.gz.sig"
cp dmg/빌도락_*_aarch64.dmg       "dmg/bildorak_$(jq -r .version ../../../tauri.conf.json)_aarch64.dmg" 2>/dev/null || true
cd -
```

주의: `cp` 가 아니라 `mv` 다 - `make-latest-json.mjs` 는 `*.app.tar.gz` 가 폴더에 정확히
하나일 때만 동작한다(둘이면 추측하지 않고 에러로 멈춘다). 서명은 파일 내용에 대한 것이라
이름을 바꿔도 유효하다.

## 5. latest.json 만들기

`scripts/make-latest-json.mjs` 가 위 `.app.tar.gz` + `.sig` 와 버전을 읽어 updater 가 기대하는
형식의 `latest.json` 을 만든다(Node 내장 모듈만 쓰는 zero-dep 스크립트). **생성된 latest.json 의
url 이 `bildorak.app.tar.gz`(ASCII) 를 가리키는지 반드시 눈으로 확인한다.**

```bash
node scripts/make-latest-json.mjs
```

기본값만으로 대부분 충분하다 — 버전은 `src-tauri/tauri.conf.json` 에서 읽고, 산출물은
`src-tauri/target/release/bundle/macos/` 에서 찾고, 결과는 레포 루트의 `latest.json` 에 쓴다.
릴리스 노트를 붙이고 싶으면:

```bash
node scripts/make-latest-json.mjs --notes "이번 버전에서 바뀐 점 요약"
```

전체 옵션은 스크립트 상단 주석 참고(`--version`, `--notes`, `--pub-date`, `--bundle-dir`, `--repo`,
`--out`). 결과 `latest.json` 예시 형태:

```json
{
  "version": "0.1.1",
  "notes": "이번 버전에서 바뀐 점 요약",
  "pub_date": "2026-08-31T12:00:00.000Z",
  "platforms": {
    "darwin-aarch64": {
      "signature": "dW50cnVzdGVkIGNvbW1lbnQ6...",
      "url": "https://github.com/gradibo/bildorak/releases/download/v0.1.1/빌도락.app.tar.gz"
    }
  }
}
```

## 6. 버전 올리기 + GitHub Release 업로드

1. 버전 번호를 세 곳 동시에 맞춘다(`commands.rs::get_app_version` 문서 참고 — 셋이 항상 같아야 한다):
   - `bildorak/package.json` 의 `version`
   - `bildorak/src-tauri/Cargo.toml` 의 `version`
   - `bildorak/src-tauri/tauri.conf.json` 의 `version`
2. 위 §3 대로 서명 빌드 → §4 대로 `latest.json` 생성.
3. 태그를 만들어 GitHub Release 를 올린다(버전 앞에 `v` 접두사 — `make-latest-json.mjs` 의 다운로드
   URL 이 이 규칙을 그대로 가정한다):

   ```bash
   gh release create v0.1.1 \
     "src-tauri/target/release/bundle/macos/빌도락.app.tar.gz" \
     "src-tauri/target/release/bundle/macos/빌도락.app.tar.gz.sig" \
     "src-tauri/target/release/bundle/dmg/"*.dmg \
     latest.json \
     --title "빌도락 v0.1.1" \
     --notes "이번 버전에서 바뀐 점 요약"
   ```
4. 확인: `https://github.com/gradibo/bildorak/releases/latest/download/latest.json` 을 브라우저나
   `curl` 로 열어 방금 올린 내용이 그대로 보이는지 본다. 기존에 설치된 구버전 앱을 열면(자동 확인이
   켜져 있으면) 곧 업데이트 모달이 뜬다 — 설정 화면에서 수동으로 껐다 켜서 재확인해도 된다.

## 7. 문제 해결

- **모달이 안 떠요**: 설정 → "자동 업데이트 확인" 토글이 켜져 있는지, GitHub Release 에
  `latest.json` asset 이 정말 붙어 있는지, 새 버전이 `tauri.conf.json` 현재 버전보다 실제로 높은지
  확인한다.
- **서명 검증 실패**: `tauri.conf.json` 의 `pubkey` 가 지금 쓰는 개인키와 짝이 맞는 공개키인지
  확인한다(키를 재발급했는데 `pubkey` 를 안 바꾼 경우 흔히 발생).
- **빌드에 `.sig` 가 안 생겨요**: `TAURI_SIGNING_PRIVATE_KEY`(또는 `_PATH`) 환경변수 없이 빌드하면
  서명 없이 그냥 빌드만 된다 — §3 명령을 그대로 다시 실행한다.
