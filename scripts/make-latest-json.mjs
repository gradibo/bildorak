#!/usr/bin/env node
// make-latest-json.mjs — `npm run tauri build`(createUpdaterArtifacts: true) 산출물(.app.tar.gz +
// .app.tar.gz.sig)과 버전을 읽어 Tauri updater 가 기대하는 latest.json 을 만든다. Node 내장 모듈만
// 쓴다(zero-dep) — package.json 에 새 의존성을 추가하지 않는다.
//
// latest.json 의 정확한 필드 이름(version/notes/pub_date/platforms/{signature,url})은 추측이 아니라
// 설치된 tauri-plugin-updater 크레이트 소스(RemoteRelease 의 Deserialize impl, updater.rs)를 직접
// 읽어 확인했다 — snake_case 그대로이고 camelCase 별칭이 없다(CLAUDE.md §16 실측 원칙).
//
// 사용법(docs/RELEASING.md 참고):
//   node scripts/make-latest-json.mjs
//   node scripts/make-latest-json.mjs --notes "이번 릴리스 노트" --out ./latest.json
//
// 옵션(전부 선택):
//   --version <semver>   기본값: src-tauri/tauri.conf.json 의 "version"
//   --notes <text>       기본값: 없음(필드 생략, 옵션 필드라 생략 가능)
//   --pub-date <rfc3339> 기본값: 지금 시각(UTC)
//   --bundle-dir <path>  기본값: src-tauri/target/release/bundle/macos
//   --repo <owner/name>  기본값: gradibo/bildorak (updater endpoint 와 동일 저장소여야 함)
//   --out <path>         기본값: 레포 루트의 latest.json

import { readFileSync, writeFileSync, readdirSync } from "node:fs";
import { join, basename, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function fail(message) {
  console.error(`[make-latest-json] ${message}`);
  process.exit(1);
}

function parseCliArgs() {
  const { values } = parseArgs({
    options: {
      version: { type: "string" },
      notes: { type: "string" },
      "pub-date": { type: "string" },
      "bundle-dir": { type: "string" },
      repo: { type: "string" },
      out: { type: "string" },
    },
    allowPositionals: false,
  });
  return values;
}

function defaultVersion() {
  const confPath = join(REPO_ROOT, "src-tauri", "tauri.conf.json");
  const conf = JSON.parse(readFileSync(confPath, "utf8"));
  if (!conf.version) fail(`${confPath} 에 "version" 필드가 없어요. --version 으로 직접 넘겨주세요.`);
  return conf.version;
}

/** bundleDir 에서 "*.app.tar.gz"(서명 파일 ".sig" 는 제외) 하나를 찾는다 — createUpdaterArtifacts 로
 * 빌드하면 이 이름의 파일이 정확히 하나만 나온다(macOS, 단일 앱 기준). 여러 개거나 없으면 사람이 알아볼
 * 수 있는 에러로 멈춘다(추측해서 아무거나 고르지 않는다). */
function findTarGz(bundleDir) {
  let entries;
  try {
    entries = readdirSync(bundleDir);
  } catch (e) {
    fail(
      `번들 폴더를 열지 못했어요: ${bundleDir} (${e.message}) — 먼저 ` +
        `TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/bildorak-updater.key) npm run tauri build 를 실행했는지 확인해 주세요.`,
    );
  }
  const candidates = entries.filter((name) => name.endsWith(".app.tar.gz"));
  if (candidates.length === 0) {
    fail(`${bundleDir} 에 *.app.tar.gz 가 없어요 — createUpdaterArtifacts 빌드 산출물을 찾지 못했어요.`);
  }
  if (candidates.length > 1) {
    fail(`${bundleDir} 에 *.app.tar.gz 후보가 ${candidates.length}개예요 — 하나만 남기고 정리한 뒤 다시 실행해 주세요: ${candidates.join(", ")}`);
  }
  return join(bundleDir, candidates[0]);
}

function platformTarget() {
  if (process.platform !== "darwin") {
    fail(`이 스크립트는 macOS 빌드 산출물 전용이에요(현재 platform: ${process.platform}).`);
  }
  return `darwin-${process.arch === "arm64" ? "aarch64" : "x86_64"}`;
}

function main() {
  const args = parseCliArgs();
  const version = args.version ?? defaultVersion();
  const repo = args.repo ?? "gradibo/bildorak";
  const bundleDir = args["bundle-dir"] ?? join(REPO_ROOT, "src-tauri", "target", "release", "bundle", "macos");
  const outPath = args.out ?? join(REPO_ROOT, "latest.json");
  const pubDate = args["pub-date"] ?? new Date().toISOString();

  const tarGzPath = findTarGz(bundleDir);
  const sigPath = `${tarGzPath}.sig`;
  let signature;
  try {
    signature = readFileSync(sigPath, "utf8").trim();
  } catch (e) {
    fail(`서명 파일을 못 찾았어요: ${sigPath} (${e.message}) — createUpdaterArtifacts:true 로 빌드했는지, TAURI_SIGNING_PRIVATE_KEY 를 설정했는지 확인해 주세요.`);
  }
  if (!signature) fail(`서명 파일이 비어 있어요: ${sigPath}`);

  const target = platformTarget();
  const assetName = basename(tarGzPath);
  const url = `https://github.com/${repo}/releases/download/v${version}/${encodeURIComponent(assetName)}`;

  const manifest = {
    version,
    ...(args.notes ? { notes: args.notes } : {}),
    pub_date: pubDate,
    platforms: {
      [target]: { signature, url },
    },
  };

  writeFileSync(outPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");

  console.log(`[make-latest-json] ${outPath} 생성 완료`);
  console.log(`  version: ${version}`);
  console.log(`  platform: ${target}`);
  console.log(`  asset: ${assetName}`);
  console.log(`  url: ${url}`);
  console.log(`  signature: ${signature.slice(0, 12)}… (${signature.length}자)`);
}

main();
