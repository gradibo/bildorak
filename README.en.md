# bildorak (빌도락)

[한국어](README.md) | **[English](README.en.md)**

**Build, sign, and get your mobile apps store-ready - locally, from a simple desktop app.**

bildorak is a desktop GUI that builds Flutter apps, manages signing keys, and produces
store-ready builds (Android `.aab` / iOS `.ipa`) on your own machine - no CI service required.
It's for people who want to get an app to the store without wrestling with command-line signing.

Free and open source (MIT). Built with Tauri 2 + React + Rust.

> ⚠️ Early-stage / portfolio project. macOS is the primary target (iOS builds require Xcode).

## Features

- **Local builds** - Android debug (`apk`), iOS simulator debug, Android release (`aab`),
  iOS release (`ipa`, App Store export)
- **Signing, made simple**
  - Scan your computer to find Android keystores and Apple `.p8` keys automatically
  - Keystore passwords are stored only in the macOS Keychain - never in files or logs
  - Auto-fill passwords from your project's `key.properties`
  - Cloud-synced keys (Google Drive, iCloud, ...) are detected, with a heads-up when a
    download is needed first
  - Keeps a safe copy of your keystore in an app-managed vault (your original is never moved)
  - See certificate expiry and fingerprint at a glance
- **Per-app checklist** - for each project, see what's ready (signing / upload) and what's left
- **Store-ready builds for manual upload** - Android `.aab` → Play Console, iOS `.ipa` →
  Transporter / `altool`
- **Build history and completion notifications**, dark mode, Korean/English UI

## CLI - for AI agents and automation

`bildorak-cli` ships alongside the GUI, sharing the exact same engine and data. AI coding
agents (like Claude Code) and CI scripts can drive bildorak straight from the terminal.

```bash
bildorak-cli apps                               # list registered apps
bildorak-cli build <app> --target ios-release   # signed, store-ready build
bildorak-cli status <app>                       # release-readiness checklist
bildorak-cli keys                               # signing keys (passwords are never printed)
bildorak-cli doctor                             # environment check (Flutter/Xcode/Android SDK)
```

- Every command takes `--json` for structured, machine-readable output
- Exit code contract: `0` on success, non-zero on failure - safe for scripts and CI
- The intended split: a human registers signing keys once in the GUI; an AI agent runs
  builds repeatedly through the CLI

## Requirements

- macOS (iOS builds need Xcode; Android builds work wherever Flutter runs)
- [Flutter](https://flutter.dev) installed
- [Rust](https://www.rust-lang.org/) + Node.js to build bildorak itself from source

## Build from source

```bash
npm install
npm run tauri dev      # run in development
npm run tauri build    # produce a distributable
cargo build --release --manifest-path src-tauri/Cargo.toml   # CLI: src-tauri/target/release/bildorak-cli
```

## Safety

bildorak never uploads your signing keys or passwords anywhere - everything runs locally:

- Passwords live only in the macOS Keychain.
- Keystores are **copied** (never moved) into an app-managed vault; your originals stay put.
- iOS signing reuses your existing Xcode certificates via `flutter build ipa`.

## License

MIT © 2026 Gradibo. See [LICENSE](LICENSE).

Made by [Gradibo](https://github.com/gradibo), a solo maker studio. bildorak was built and
tested on our own shipping apps before release.

## Contributing

Issues and pull requests are welcome. This is a small project made for fun and for the community.
