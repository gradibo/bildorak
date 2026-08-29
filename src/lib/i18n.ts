// i18n.ts — 가벼운 다국어 사전(설정 화면 "언어" 옵션). 무거운 라이브러리 대신 평범한 Record 하나로
// 충분한 범위만 다룬다. ⚠️ 번역 대상은 설정 화면 자체 + 좌측 네비 + 일부 최상위 섹션 제목뿐이다
// (settings-context.tsx 문서 참고) — 프로젝트 카드, 점검 결과(Rust 쪽에서 한국어로 내려옴), 에러
// 문구 등은 이번 범위 밖이라 이 사전에 없다. 새 화면을 번역 대상에 추가하려면 키를 여기 추가하고
// useSettings().t(key) 로 쓴다.

export const DICTIONARY = {
  ko: {
    "nav.projects": "프로젝트",
    "nav.settings": "설정",

    "app.eyebrow": "로컬 빌드 준비 점검",
    "app.title": "등록된 앱",

    "signing.checklistTitle": "출시 준비 체크리스트",
    "signing.signRow": "서명(도장)",
    "signing.uploadRow": "업로드(출입증)",

    "settings.eyebrow": "빌도락 설정",
    "settings.title": "설정",
    "settings.flutterSdk.label": "Flutter SDK 경로",
    "settings.flutterSdk.hint": "설정하지 않으면 시스템 PATH의 flutter 명령을 그대로 사용해요.",
    "settings.flutterSdk.detect": "자동 감지",
    "settings.flutterSdk.detecting": "찾는 중…",
    "settings.flutterSdk.checking": "확인하는 중…",
    "settings.language.label": "언어",
    "settings.theme.label": "테마",
    "settings.theme.system": "시스템",
    "settings.theme.light": "라이트",
    "settings.theme.dark": "다크",
    "settings.notifications.label": "빌드 완료 알림",
    "settings.notifications.hint": "빌드가 끝나면 macOS 알림을 보여줘요.",
    "settings.vault.label": "서명키 보관함 위치",
    "settings.vault.openFinder": "Finder에서 열기",
    "settings.cli.label": "CLI / 자동화",
    "settings.cli.intro": "Claude Code 같은 AI 에이전트나 스크립트가 터미널에서 빌도락을 쓸 수 있어요.",
    "settings.cli.copy": "복사",
    "settings.cli.copied": "복사됨",
    "settings.cli.loadError": "CLI 명령 목록을 불러오지 못했어요.",
    "settings.cli.binaryHint":
      "아직 별도로 배포하는 바이너리는 없어요 — 소스에서 직접 빌드해요: cargo build --release 실행 후 " +
      "src-tauri/target/release/bildorak-cli 에서 찾을 수 있어요.",
    "settings.about.label": "정보",
    "settings.about.version": "버전",
    "settings.about.github": "GitHub 저장소",
  },
  en: {
    "nav.projects": "Projects",
    "nav.settings": "Settings",

    "app.eyebrow": "Local build readiness check",
    "app.title": "Registered apps",

    "signing.checklistTitle": "Release readiness checklist",
    "signing.signRow": "Signing",
    "signing.uploadRow": "Upload",

    "settings.eyebrow": "bildorak settings",
    "settings.title": "Settings",
    "settings.flutterSdk.label": "Flutter SDK path",
    "settings.flutterSdk.hint": "If not set, bildorak uses the `flutter` command from your system PATH.",
    "settings.flutterSdk.detect": "Auto-detect",
    "settings.flutterSdk.detecting": "Detecting…",
    "settings.flutterSdk.checking": "Checking…",
    "settings.language.label": "Language",
    "settings.theme.label": "Theme",
    "settings.theme.system": "System",
    "settings.theme.light": "Light",
    "settings.theme.dark": "Dark",
    "settings.notifications.label": "Build completion notifications",
    "settings.notifications.hint": "Shows a macOS notification when a build finishes.",
    "settings.vault.label": "Signing key vault location",
    "settings.vault.openFinder": "Reveal in Finder",
    "settings.cli.label": "CLI / Automation",
    "settings.cli.intro": "AI agents like Claude Code, or scripts, can drive bildorak from the terminal.",
    "settings.cli.copy": "Copy",
    "settings.cli.copied": "Copied",
    "settings.cli.loadError": "Couldn't load the CLI command list.",
    "settings.cli.binaryHint":
      "There's no separate distributed binary yet — build it from source: run cargo build --release, then " +
      "find it at src-tauri/target/release/bildorak-cli.",
    "settings.about.label": "About",
    "settings.about.version": "Version",
    "settings.about.github": "GitHub repository",
  },
} as const;

export type TranslationKey = keyof typeof DICTIONARY.ko;
