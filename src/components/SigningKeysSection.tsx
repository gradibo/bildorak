// SigningKeysSection.tsx — ProjectCard 안, preflight 섹션 아래에 붙는 "출시 준비" 섹션.
// 앱별 체크리스트 구조(확정된 재구성) — 열쇠를 종류별 창고로 나열하던 1차 골격 대신, "이
// 앱(project_id)이 서명·업로드 준비가 됐는지"를 2행 체크리스트(서명(도장)/업로드(출입증))로 먼저
// 보여준다. 저장 위치는 그대로다 — keychain 한 곳(변경 없음), 새 비밀 저장 경로·새 keytool 호출 없음.
// 이 파일은 화면/데이터 "조립"만 바꾼다 — 실제 등록·스캔·keychain IO 는 여전히 signing.rs/key_scan.rs 가
// 담당하고 여기서는 그 결과를 다르게 배치해 보여줄 뿐이다.
//
// 체크리스트 두 행:
//   - 서명(도장): 이 프로젝트에 연결된 Android keystore 서명키(linkedKeys 중 kind ==
//     "android_keystore" 첫 번째, "대표 서명키") 의 androidSigning(keychain 비밀번호 등록 여부) 유무로
//     판정한다. iOS 서명은 이번 범위 밖(model.rs::BuildTarget 문서와 동일 — 프로젝트 자체 Xcode 설정을
//     따른다)이라 이 행은 Android 전용이다.
//   - 업로드(출입증): App Store Connect API 키(.p8)는 계정 단위라 앱별로 강제 매핑하지 않는다 —
//     store_keys.json 에 "발견 기록"된 전체 개수만 정보로 보여주고, 실제 업로드는 아직 기능이 없다
//     (로드맵 #6, coming-soon 표시만 — 여기서 새로 구현하지 않는다).
// 체크리스트 "서명" 행도 "다른 앱에서도 사용 중" 표시와 만료 pill 을 보여준다. 단 pill 은 expiresAt 이
// 있는 키만(signingKeyExpiryStatus 가 "unknown" 이면 숨김) — Android 서명키는 expiresAt 이 없어 pill
// 대신 실제 인증서 만료를 아래 메타 텍스트("인증서 만료 …", androidCertMetaLine)로 노출한다.
// (otherLinkedAppNames/signingKeyExpiryLabel/EXPIRY_PILL_CLASS — "그 외 연결된 키" 목록과 동일 헬퍼).
// 체크리스트에 요약되지 않는 나머지(대표 서명키 외 이 앱에 연결된 다른 키)는 "그 외 연결된 키"에,
// 등록된 어떤 앱과도 안 묶이는 스캔 결과는 FoundKeysPanel 결과 목록에 그대로 남는다 — 재구성 전
// 정보 중 사라지는 것은 없다.
//
// "내 컴퓨터에서 찾기" 결과(FoundKeysPanel)는 모달(팝업)로 띄운다 — 체크리스트 "서명" 행의 [내
// 컴퓨터에서 찾기 · 등록]과 이 섹션 하단의 [내 컴퓨터에서 찾기] 버튼이 둘 다 같은 모달(scanModalOpen)을
// 연다. 예전엔 이 결과를 섹션 맨 아래 인라인으로 그려서 스크롤을 한참 내려야 보였다(사용성 피드백) —
// 지금은 모달 안에서 스캔·등록·(필요시) 비밀번호 입력까지 한 화면에서 끝난다
// (Modal.tsx, FoundKeysPanel 문서 참고). 이미 등록된 키 개요("그 외 연결된 키")는 자주 훑어보는
// 정보라 그대로 인라인에 남긴다.
//
// "빌도락에서 제거"(구 "완전히 삭제")도 이제 window.confirm 대신 같은 Modal 을 재사용한다(Tauri 환경
// window.confirm 불안정, 리뷰 지적) — confirmRemove 상태 하나를 체크리스트/"그 외 연결된 키" 두 행이
// 공유한다(scanModalOpen 과 같은 패턴). 이름을 바꾼 이유: remove_signing_key(commands.rs)는 빌도락
// 등록 기록과 keychain 비밀번호만 지우고 원본 keystore 파일은 건드리지 않는데, "완전히 삭제"라는
// 라벨은 파일까지 지우는 것처럼 읽혀 실제보다 위험해 보였다(리뷰 지적). 그래서 signing-key-actions
// 안의 세 버튼도 위험도로 나눈다 — 서명 비밀번호 등록/연결 해제는 btn-text-secondary(중립), 제거만
// btn-danger-text + signing-key-actions-danger(구분선으로 오른쪽에 분리)로 남긴다(App.css 참고).
//
// FoundKeysPanel 의 "이 앱 것 같아요"(구 "이 앱 추천") 하이라이트는 appId 일치 외에 파일명/경로에
// 프로젝트 폴더 이름이 포함되는지도 본다(foundKeyMatchesProject, copy.ts) — 홑파일 keystore(옆에
// build.gradle 이 없어 appId 를 못 구하는 경우, 예: myapp-upload-keystore.jks)도 걸리게 하기
// 위해서다(리뷰 지적). 여전히 힌트일 뿐이라 자동 등록은 하지 않는다.
//
// 서명키 "관리"(등록·보기·연결·만료 확인)와 실제 "서명 + 스토어 업로드" 모두 무료다(commands.rs::
// signing_base_dir, 무료 오픈소스라 게이트 없음) — 그래서 이 컴포넌트에는 Pro 잠금 분기가 없다.
//
// ⚠️ 여기서도 키의 비밀 내용은 절대 다루지 않는다 — SigningKeyRecord 자체가 겉정보(종류·이름·만료일)
// 뿐이라 화면에 표시할 것도 그것뿐이다.

import { useEffect, useRef, useState } from "react";
import {
  autofillAndroidSigning,
  getProjectAppId,
  importFoundAndroidSigning,
  inspectKeySource,
  linkSigningKey,
  listFoundStoreKeys,
  pickSigningKeyFile,
  registerAndroidSigning,
  registerFoundStoreKey,
  registerSigningKey,
  removeSigningKey,
  revealSigningKeyInFinder,
  scanSigningKeys,
  unlinkSigningKey,
} from "../lib/api";
import {
  androidCertMetaLine,
  cloudKindLabelFromPath,
  foundAndroidKeyAppIdLabel,
  foundAndroidKeyAppNameGuess,
  foundKeyMatchesProject,
  looksLikeCloudPath,
  projectFolderName,
  signingKeyExpiryLabel,
  signingKeyExpiryStatus,
  vaultStatusLine,
  type SigningKeyExpiryStatus,
} from "../lib/copy";
import {
  P8_SUBTYPE_LABEL,
  SIGNING_KEY_KIND_LABEL,
  type FoundKey,
  type FoundKeyKind,
  type FoundStoreKeyRecord,
  type KeySourceInfo,
  type SigningKeyRecord,
} from "../lib/types";
import { useSettings } from "../lib/settings-context";
import { CheckStatusIcon, SigningKeyKindIcon, SpinnerIcon } from "./Icons";
import { Modal } from "./Modal";

const EXPIRY_PILL_CLASS: Record<SigningKeyExpiryStatus, string> = {
  valid: "pill-ok",
  no_expiry: "pill-ok",
  expiring_soon: "pill-warn",
  expired: "pill-crit",
  unknown: "pill-idle",
};

/**
 * Android keystore 서명키 한 건의 release 자동 서명 비밀번호 등록 폼(다음 단계) — 접었다 펼 수 있는
 * 형태로, key_alias/store 비밀번호/key 비밀번호를 입력받아 registerAndroidSigning 을 부른다. 비밀번호는
 * 저장 성공/실패와 무관하게 제출 직후 폼 상태에서 지운다(화면·상태에 평문 잔류 최소화 — 확정된
 * 설계 보안 원칙). 비밀번호 입력은 항상 type="password".
 */
function AndroidSigningForm({
  keyId,
  currentAlias,
  autoOpen,
  onSaved,
}: {
  keyId: string;
  currentAlias?: string;
  /** 찾기(FoundKeysPanel)에서 "등록"을 눌렀는데 key.properties 에 비밀번호가 없어 폴백했을 때만 true —
   * 폼을 자동으로 펼치고 저장소 비밀번호 입력칸에 포커스를 준다("경로·alias 만 pre-fill + 비번 필드
   * 포커스", FoundKeysPanel 문서 참고). */
  autoOpen?: boolean;
  onSaved: (key: SigningKeyRecord) => void;
}) {
  const [open, setOpen] = useState(false);
  const [alias, setAlias] = useState(currentAlias ?? "");
  const [storePassword, setStorePassword] = useState("");
  const [keyPassword, setKeyPassword] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const storePasswordRef = useRef<HTMLInputElement | null>(null);

  // autoOpen 이 false→true 로 바뀌는 순간에만 한 번 폼을 펼치고 alias 를 채운다 — 이미 사용자가 직접
  // 입력 중인 값은 덮어쓰지 않는다(prev 가 비어 있을 때만 채움).
  useEffect(() => {
    if (!autoOpen) return;
    setOpen(true);
    setAlias((prev) => prev || currentAlias || "");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoOpen]);

  // 위 effect 가 open 을 true 로 바꾼 "다음" 렌더에서 실제 input 이 DOM 에 그려진 뒤에야 포커스를 줄 수
  // 있다 — 같은 effect 안에서 setOpen 직후 바로 focus() 를 부르면 아직 안 그려진 input 이라 먹지 않는다.
  useEffect(() => {
    if (open && autoOpen) {
      storePasswordRef.current?.focus();
    }
  }, [open, autoOpen]);

  const handleSave = async () => {
    if (saving) return;
    setSaving(true);
    setError(null);
    try {
      const updated = await registerAndroidSigning(keyId, alias.trim(), storePassword, keyPassword);
      onSaved(updated);
      setOpen(false);
    } catch (e) {
      setError(typeof e === "string" ? e : "서명 비밀번호를 저장하지 못했어요.");
    } finally {
      // 성공/실패와 무관하게 비밀번호는 폼에 남기지 않는다 — 저장은 이미 keychain 이 담당했거나(성공)
      // 실패했으니 다시 입력받는다.
      setStorePassword("");
      setKeyPassword("");
      setSaving(false);
    }
  };

  if (!open) {
    return (
      <button type="button" className="btn-text-secondary" onClick={() => setOpen(true)}>
        {currentAlias ? "서명 비밀번호 다시 등록" : "release 자동 서명 비밀번호 등록"}
      </button>
    );
  }

  return (
    <div className="android-signing-form">
      {/* autoOpen 은 항상 "자동 채움을 시도했는데 못 찾았다"는 신호와만 함께 켜진다(checklistManualEntry/
          manualPasswordFor/manualEntry 셋 다 autofill 실패·에러 분기에서만 채워진다) — 그래서 이 문구를
          autoOpen 하나로만 게이팅해도 안전하다(리뷰 지적 — 자동 채움 시도 자체가 안 보였다). */}
      {autoOpen && <p className="signing-key-meta">비밀번호를 자동으로 못 찾았어요 — 직접 입력해주세요.</p>}
      <label className="android-signing-field">
        키 별칭(alias)
        <input
          type="text"
          value={alias}
          onChange={(e) => setAlias(e.target.value)}
          placeholder="예: release"
          autoComplete="off"
        />
      </label>
      <label className="android-signing-field">
        저장소(store) 비밀번호
        <input
          ref={storePasswordRef}
          type="password"
          value={storePassword}
          onChange={(e) => setStorePassword(e.target.value)}
          autoComplete="off"
        />
      </label>
      <label className="android-signing-field">
        키(key) 비밀번호
        <input
          type="password"
          value={keyPassword}
          onChange={(e) => setKeyPassword(e.target.value)}
          autoComplete="off"
        />
      </label>
      {error && <p className="android-signing-error">{error}</p>}
      <p className="signing-key-meta">비밀번호는 이 기기의 macOS 키체인에만 저장돼요(파일로 남기지 않아요).</p>
      <div className="card-actions">
        <button
          type="button"
          className="btn btn-outline"
          disabled={saving || !alias.trim() || !storePassword || !keyPassword}
          onClick={() => void handleSave()}
        >
          {saving && <SpinnerIcon />}
          {saving ? "저장하는 중…" : "keychain에 안전하게 저장"}
        </button>
        <button type="button" className="btn-danger-text" disabled={saving} onClick={() => setOpen(false)}>
          취소
        </button>
      </div>
    </div>
  );
}

function isAndroidFoundKey(
  key: FoundKey,
): key is FoundKey & { kind: Extract<FoundKeyKind, { type: "android_keystore" }> } {
  return key.kind.type === "android_keystore";
}

function isP8FoundKey(key: FoundKey): key is FoundKey & { kind: Extract<FoundKeyKind, { type: "apple_p8" }> } {
  return key.kind.type === "apple_p8";
}

/**
 * "내 컴퓨터에서 찾기" 결과 — scan_signing_keys 로 고정 경로(스캔 규칙)를 스캔해 찾은 Android
 * keystore/.p8 후보를 보여준다. 스캔 트리거(scanning/scanned/foundKeys)는 SigningKeysSection(부모) 이
 * 들고 있다 — 체크리스트 "서명" 행의 [내 컴퓨터에서 찾기] 버튼과 이 패널의 버튼이 같은 스캔 결과를
 * 공유해야 해서다(버튼이 둘이어도 스캔은 한 번의 상태). 이 컴포넌트는 그 결과를 "렌더"하고, 후보별
 * 등록/기록 같은 개별 액션만 자기 상태로 관리한다.
 *
 * recommendedAppId(체크리스트가 이미 확인해 둔 이 프로젝트의 applicationId) 또는 recommendedFolderName
 * (프로젝트 폴더 이름)과 일치하는 Android 후보를 목록 맨 앞으로 정렬 + "이 앱 것 같아요" 배지를 붙인다
 * (foundKeyMatchesProject, copy.ts) — 여러 앱을 등록해 둔 상태에서 스캔 결과가 섞여 나올 때 어떤 걸
 * 눌러야 할지 바로 보이게 하기 위함이다(정렬은 이 화면 표시만 바꿀 뿐 스캔 결과 자체(foundKeys)는
 * 건드리지 않는다). appId 매칭만으론 홑파일 keystore(옆에 build.gradle 이 없어 appId 를 못 구하는
 * myapp-upload-keystore.jks 같은 경우)가 전혀 안 걸렸다(리뷰 지적) — 폴더 이름 매칭을 더해 넓혔다.
 *
 * 비밀번호 "값"은 이 컴포넌트 어디에도 오지 않는다(scan_signing_keys/import_found_android_signing
 * 반환값 자체에 없음, commands.rs/key_scan.rs 주석 참고). Android 항목의 "등록" 버튼은 항상
 * import_found_android_signing 을 부른다 — 서버가 key.properties 를 다시 읽어(스캔 시점의
 * passwordsAvailable 을 그대로 믿지 않음) 비밀번호가 있으면 keychain 으로 자동 이관(imported:true)하고,
 * 없으면 등록·연결만 해 둔 채 imported:false 를 돌려준다. 그 경우 이 목록의 해당 항목 바로 아래에
 * AndroidSigningForm 을 펼치고 포커스한다(manualPasswordFor, found.path 로 항목을 식별) — 예전엔 부모
 * (SigningKeysSection)의 체크리스트/"그 외 연결된 키" 쪽 폼을 대신 열었지만, 이 패널 전체가 모달 안으로
 * 옮겨오면서(파일 상단 문서 참고) 그쪽 폼은 모달에 가려 사용자 눈에 안 띄는 문제가 있었다. 지금은 이
 * 패널이 자기 상태로 들고 있다가 비밀번호 등록 성공(onSaved) 시 지운다 — 모달을 닫았다 다시 열면
 * FoundKeysPanel 자체가 새로 마운트되니(Modal.tsx 문서 참고) 그때도 자동으로 비워진다.
 *
 * 링크된 키가 이미 signingComplete(androidSigning 존재)면 이 "등록" 버튼 대신 체크리스트/"그 외 연결된
 * 키"와 같은 접힌 AndroidSigningForm("다시 등록")만 보여준다 — "이미 등록됨"과 "비밀번호 직접 입력
 * 필요" 배지가 동시에 뜨던 모순을 없애기 위해서다. 배지·폼 판정은 이제 매 항목
 * 마다 matchedKey(=found.path 와 일치하는 SigningKeyRecord)의 androidSigning 유무로만 정해진다
 * (androidResults.map 내부 needsPassword/showPasswordForm 참고) — passwordsAvailable(스캔 시점 key.
 * properties 존재 여부)은 아직 링크 전인 후보에만 쓴다.
 *
 * .p8 "기록" 개수는 체크리스트 "업로드" 행에 계정 수준으로 표시된다 — foundStoreKeys/onStoreKeyRecorded
 * 도 부모가 들고 있다(단일 출처, 이 패널의 "기록됨" 배지와 체크리스트 카운트가 같은 데이터를 본다).
 */
function FoundKeysPanel({
  projectId,
  allKeys,
  onRegistered,
  onUpdated,
  scanning,
  scanned,
  foundKeys,
  scanError,
  onScan,
  recommendedAppId,
  recommendedFolderName,
  foundStoreKeys,
  onStoreKeyRecorded,
}: {
  projectId: string;
  allKeys: SigningKeyRecord[];
  onRegistered: (key: SigningKeyRecord) => void;
  onUpdated: (key: SigningKeyRecord) => void;
  scanning: boolean;
  scanned: boolean;
  foundKeys: FoundKey[];
  scanError: string | null;
  onScan: () => void;
  /** 이 프로젝트의 applicationId(체크리스트가 이미 조회해 둔 값) — 있으면 일치하는 Android 후보를
   * 우선 정렬/하이라이트한다. 못 구했으면(null) 정렬 없이 스캔이 돌려준 순서(최신순) 그대로 보여준다. */
  recommendedAppId: string | null;
  /** 이 프로젝트 폴더 이름(copy.ts::projectFolderName) — recommendedAppId 로 못 거른 홑파일 keystore도
   * 파일명/경로에 이 이름이 들어 있으면 힌트로 잡는다(foundKeyMatchesProject). 2자 이하면 매칭 자체를
   * 하지 않는다(오탐 방지, foundKeyMatchesProject 문서 참고). */
  recommendedFolderName: string;
  /** 계정 수준 .p8 "발견 기록" 전체 — 이미 기록된 후보를 "기록됨"으로 표시하는 데 쓴다. */
  foundStoreKeys: FoundStoreKeyRecord[];
  onStoreKeyRecorded: (record: FoundStoreKeyRecord) => void;
}) {
  const [actionError, setActionError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const [importingPath, setImportingPath] = useState<string | null>(null);
  const [recordingPath, setRecordingPath] = useState<string | null>(null);
  // 찾은 Android 키를 "등록"했는데 비밀번호를 못 찾았을 때만(imported:false) 채워진다 — 어느 항목(path
  // 로 식별) 바로 아래에 AndroidSigningForm 을 펼칠지 결정한다. keyId/alias 는 그 폼에 그대로 넘긴다
  // (runImport 참고).
  const [manualPasswordFor, setManualPasswordFor] = useState<{ path: string; keyId: string; alias: string } | null>(
    null,
  );
  // "등록" 클릭 직후 inspectKeySource 로 확인 중인 항목(path) — 스캔 시점이 아니라 클릭 시점에
  // 매번 다시 확인한다(TOCTOU 방지, import_android_signing 이 passwordsAvailable 을 다시 읽는 것과
  // 같은 이유 — 스캔과 클릭 사이 다운로드가 끝났을 수도, 반대로 파일이 옮겨졌을 수도 있다).
  const [checkingCloudPath, setCheckingCloudPath] = useState<string | null>(null);
  // inspectKeySource 결과 isCloud 이면(다운로드 여부 무관) 여기 채워져 클라우드 확인 Modal 이 뜬다 —
  // null 이면 모달이 닫혀 있다(scanModalOpen/confirmRemove 와 같은 패턴, 파일 상단 문서 참고).
  const [cloudCheck, setCloudCheck] = useState<{ found: FoundKey; info: KeySourceInfo } | null>(null);
  const [revealingPath, setRevealingPath] = useState<string | null>(null);

  const runImport = async (found: FoundKey) => {
    setImportingPath(found.path);
    setActionError(null);
    setSuccessMessage(null);
    try {
      const result = await importFoundAndroidSigning(found.path, projectId);
      const alreadyKnown = allKeys.some((k) => k.id === result.key.id);
      if (alreadyKnown) onUpdated(result.key);
      else onRegistered(result.key);
      if (result.imported) {
        setSuccessMessage(`${result.key.displayName} — ✓ 등록됨 · key.properties 에서 비밀번호를 자동으로 찾아 연결했어요.`);
        setManualPasswordFor(null);
      } else if (result.key.kind === "android_keystore") {
        // 인접 key.properties 에 비밀번호가 없었을 때 — handleAddKey 와 같은 패턴으로 이 프로젝트
        // 자체의 key.properties 자동 채움을 한 번 더 시도한다(리뷰 지적 — 모달 [등록] 경로가 이 채움을
        // 안 타서 myapp/otherapp 처럼 storeFile 이 이 keystore 를 정확히 가리키는 프로젝트에서도
        // 수동 폼으로 떨어졌다).
        try {
          const autofillResult = await autofillAndroidSigning(result.key.id, projectId);
          onUpdated(autofillResult.key);
          if (autofillResult.imported) {
            setSuccessMessage(
              `${autofillResult.key.displayName} — ✓ 등록됨 · key.properties 에서 비밀번호를 자동으로 찾아 연결했어요.`,
            );
            setManualPasswordFor(null);
          } else {
            setManualPasswordFor({ path: found.path, keyId: result.key.id, alias: autofillResult.keyAlias ?? "" });
          }
        } catch {
          // 자동 채움 시도 자체가 실패해도(드묾) 등록은 이미 끝났다 — 조용히 수동 입력으로 넘어간다.
          setManualPasswordFor({ path: found.path, keyId: result.key.id, alias: result.keyAlias ?? "" });
        }
      } else {
        setManualPasswordFor({ path: found.path, keyId: result.key.id, alias: result.keyAlias ?? "" });
      }
    } catch (e) {
      setActionError(typeof e === "string" ? e : "서명키를 가져오지 못했어요.");
    } finally {
      setImportingPath(null);
    }
  };

  /**
   * "등록" 버튼 클릭 시 실제 가져오기(runImport) 전에 원본이 클라우드 온디맨드(다운로드 전) 상태인지
   * 먼저 확인한다(inspectKeySource, stat 만 사용) — import_android_signing 이 내부적으로 거치는
   * copy_keystore_into_vault 가 최대 ~31초 재시도하다 실패하는 것을 미리 막고 즉시 안내한다(리뷰
   * 지적). 확인 자체가 실패해도(드묾) 하드 에러로 막지 않고 기존처럼 바로 가져오기를 진행한다 — 이
   * 사전 확인은 UX 개선일 뿐 가져오기 성공의 전제조건이 아니다.
   */
  const handleImportClick = async (found: FoundKey) => {
    if (importingPath || checkingCloudPath) return;
    setActionError(null);
    setSuccessMessage(null);
    setCheckingCloudPath(found.path);
    let info: KeySourceInfo | null = null;
    try {
      info = await inspectKeySource(found.path);
    } catch {
      info = null;
    } finally {
      setCheckingCloudPath(null);
    }
    if (info?.isCloud) {
      setCloudCheck({ found, info });
      return;
    }
    await runImport(found);
  };

  /** 클라우드 확인 Modal 의 [복사하고 등록] — 이미 다운로드된 클라우드 파일임을 사용자가 확인한
   * "다음"에만 실제 가져오기를 진행한다. */
  const handleConfirmCloudImport = () => {
    if (!cloudCheck) return;
    const { found } = cloudCheck;
    setCloudCheck(null);
    void runImport(found);
  };

  /** 클라우드 확인 Modal 의 [Finder에서 열기] — 온디맨드(미다운로드) 파일 위치를 Finder 에서 강조
   * 표시만 한다(다운로드 자체를 이 앱이 트리거하지 않는다 — 사용자가 직접 클라우드 앱에서 받아야
   * 한다). 실패해도(드묾) 모달은 열린 채로 두고 에러만 보여준다(재시도 가능하도록). */
  const handleRevealInFinder = async (path: string) => {
    if (revealingPath) return;
    setRevealingPath(path);
    try {
      await revealSigningKeyInFinder(path);
    } catch (e) {
      setActionError(typeof e === "string" ? e : "Finder에서 열지 못했어요.");
    } finally {
      setRevealingPath(null);
    }
  };

  const handleRecordStoreKey = async (found: FoundKey) => {
    if (!isP8FoundKey(found) || recordingPath) return;
    setRecordingPath(found.path);
    setActionError(null);
    try {
      const record = await registerFoundStoreKey(found.path, found.kind.keyId, found.kind.subtype);
      onStoreKeyRecorded(record);
    } catch (e) {
      setActionError(typeof e === "string" ? e : "스토어 키를 기록하지 못했어요.");
    } finally {
      setRecordingPath(null);
    }
  };

  const recordedPaths = new Set(foundStoreKeys.map((r) => r.path));
  // slice() 로 복사한 뒤 정렬 — foundKeys 는 부모가 들고 있는 state 라 원본을 직접 sort() 로 뒤섞으면
  // 안 된다. foundKeyMatchesProject 와 일치하는 항목만 앞으로 당기는 안정 정렬(appId 일치 + 폴더 이름
  // 힌트를 모두 본다, 아래 recommended 판정과 같은 기준) — 그 외 순서는 스캔이 돌려준 최신순 그대로
  // 유지.
  const androidResults = foundKeys
    .filter(isAndroidFoundKey)
    .slice()
    .sort((a, b) => {
      const aMatch = foundKeyMatchesProject(a, recommendedAppId, recommendedFolderName);
      const bMatch = foundKeyMatchesProject(b, recommendedAppId, recommendedFolderName);
      return aMatch === bMatch ? 0 : aMatch ? -1 : 1;
    });
  const storeKeyResults = foundKeys.filter(isP8FoundKey);
  const error = scanError ?? actionError;

  return (
    <div className="found-keys-panel">
      <div className="card-actions">
        <button type="button" className="btn btn-outline" disabled={scanning} onClick={onScan}>
          {scanning && <SpinnerIcon />}
          {scanning ? "찾는 중…" : "내 컴퓨터에서 찾기"}
        </button>
      </div>

      {error && (
        <div className="banner-error">
          <CheckStatusIcon status="fail" />
          <span>{error}</span>
        </div>
      )}
      {successMessage && <p className="banner-success">{successMessage}</p>}

      {scanned && foundKeys.length === 0 && !scanError && (
        <p className="history-empty">이 컴퓨터에서 서명키를 찾지 못했어요.</p>
      )}

      {androidResults.length > 0 && (
        <div className="found-keys-group">
          <p className="signing-section-label">서명키(Android)</p>
          <ul className="signing-key-list">
            {androidResults.map((found) => {
              // 배지 판정 — "링크됨"(matchedKey: found.path 가 등록된 SigningKeyRecord 와 일치)과
              // "비밀번호 완료"(matchedKey.androidSigning 존재)는 서로 다른 축이다. 예전엔 "이미
              // 등록됨"(=링크됨)과 "비밀번호 직접 입력 필요"(=스캔 시점 passwordsAvailable)를 각자
              // 독립 조건으로 렌더해서, 링크는 됐지만 keychain 비밀번호만 아직 없는 키가 두 배지를
              // 동시에 띄웠다("등록된 거야 아니야?" 문제). 지금은 needsPassword 를
              // "링크 전이면 passwordsAvailable, 링크 후면 !signingComplete" 한 축으로만 정해서
              // signingComplete(✓ 등록됨)와 needsPassword(비밀번호 필요)가 구조적으로 겹치지 않는다.
              const matchedKey = allKeys.find((k) => k.filePath === found.path) ?? null;
              const linked = Boolean(matchedKey);
              const signingComplete = Boolean(matchedKey?.androidSigning);
              const needsPassword = linked ? !signingComplete : !found.kind.passwordsAvailable;
              // 등록 전(!linked) 배지 — "비밀번호 필요"라는 막연한 경고 대신, 인접 key.properties 를
              // 찾았는지(keyPropertiesPath)로 "자동으로 채워질지"를 미리 알려준다(리뷰 지적 — 자동
              // 채움 과정이 안 보였다). passwordsAvailable(그 안에 실제 값이 둘 다 있는지)이 아니라
              // keyPropertiesPath 존재만 본다 — "감지했다"는 낙관적 예고일 뿐이고, 실제 값이 없으면
              // 등록 시점(runImport → import_android_signing)이 그대로 수동 입력으로 폴백한다(추측
              // 아님, 아래 배지는 그 결과를 대신 약속하지 않는다). 등록 후(linked)는 기존 needsPassword
              // (=키체인에 실제로 비밀번호가 있는지)를 그대로 쓴다 — 이미 끝난 키는 예고가 아니라 결과를
              // 보여줘야 한다.
              const hasKeyProperties = Boolean(found.kind.keyPropertiesPath);
              // ☁ 클라우드 배지(경로 문자열 힌트) — 등록 여부와 무관하게 항상 보여준다. 실제 등록
              // 가능 여부(다운로드 여부까지)의 최종 판정은 여전히 [등록] 클릭 시점의 inspectKeySource
              // 가 한다(copy.ts::looksLikeCloudPath 문서 참고).
              const isCloudPath = looksLikeCloudPath(found.path);
              // "혹시 이건가?" 하이라이트 — appId 일치(기존)뿐 아니라 파일명/경로에 이 프로젝트 폴더
              // 이름이 들어 있어도 켠다(foundKeyMatchesProject, copy.ts). 홑파일 keystore(appId 를 못
              // 구함, 옆에 build.gradle 없음)도 이걸로 걸린다 — 어디까지나 힌트라 [등록]은 그대로 눌러야
              // 한다(자동 등록 아님).
              const recommended = foundKeyMatchesProject(found, recommendedAppId, recommendedFolderName);
              // 비밀번호 폼 대상 — ①이번 세션에 "등록"을 눌렀다가 keychain 자동 이관에 실패해 폴백한
              // 경우(manualEntry, 기존 흐름 그대로 autoOpen + 포커스) ②이미 링크는 됐지만(과거 세션 등)
              // 아직 비밀번호가 없는 경우(matchedKey 만으로 판정, 클릭 없이 바로 보여준다 — 체크리스트/
              // "그 외 연결된 키"와 같은 접힌 기본 상태). signingComplete 면 이 폼은 대신 숨기고
              // signing-key-actions 쪽 접힌 AndroidSigningForm("다시 등록")만 남긴다(중복 폼 방지).
              const manualEntry =
                manualPasswordFor && manualPasswordFor.path === found.path ? manualPasswordFor : null;
              const formKeyId = manualEntry ? manualEntry.keyId : matchedKey?.id;
              const formAlias = manualEntry
                ? manualEntry.alias
                : (matchedKey?.androidSigning?.keyAlias ?? found.kind.alias ?? undefined);
              const showPasswordForm = !signingComplete && Boolean(manualEntry || matchedKey);
              return (
                <li key={found.path} className={`signing-key-item${recommended ? " signing-key-item-recommended" : ""}`}>
                  <SigningKeyKindIcon kind="android_keystore" />
                  <div className="signing-key-body">
                    <div className="signing-key-name-row">
                      <span className="signing-key-name">{foundAndroidKeyAppNameGuess(found)}</span>
                      {foundAndroidKeyAppIdLabel(found) && (
                        <span className="signing-key-meta">{foundAndroidKeyAppIdLabel(found)}</span>
                      )}
                      {recommended && <span className="pill pill-ok">이 앱 것 같아요</span>}
                      {signingComplete && <span className="pill pill-ok">✓ 등록됨</span>}
                      {isCloudPath && <span className="pill pill-info">☁ {cloudKindLabelFromPath(found.path)}</span>}
                      {linked ? (
                        needsPassword && <span className="pill pill-warn">비밀번호 필요</span>
                      ) : hasKeyProperties ? (
                        <span
                          className="pill pill-info"
                          title="옆의 key.properties 에서 자동으로 채워요 — 직접 입력 안 해도 돼요"
                        >
                          🔑 비밀번호 자동 감지됨
                        </span>
                      ) : (
                        <span className="pill pill-warn">비밀번호 직접 입력 필요</span>
                      )}
                    </div>
                    <p className="signing-key-meta">{found.path}</p>
                    <p className="signing-key-meta">발견일 {found.modified}</p>
                    <div className="signing-key-actions">
                      {signingComplete && matchedKey ? (
                        <AndroidSigningForm
                          keyId={matchedKey.id}
                          currentAlias={matchedKey.androidSigning?.keyAlias}
                          onSaved={onUpdated}
                        />
                      ) : (
                        <button
                          type="button"
                          className="btn btn-outline"
                          disabled={importingPath === found.path || checkingCloudPath === found.path}
                          onClick={() => void handleImportClick(found)}
                        >
                          {(importingPath === found.path || checkingCloudPath === found.path) && <SpinnerIcon />}
                          {importingPath === found.path
                            ? "가져오는 중…"
                            : checkingCloudPath === found.path
                              ? "확인하는 중…"
                              : "등록"}
                        </button>
                      )}
                    </div>
                    {showPasswordForm && formKeyId && (
                      <AndroidSigningForm
                        keyId={formKeyId}
                        currentAlias={formAlias}
                        autoOpen={Boolean(manualEntry)}
                        onSaved={(key) => {
                          onUpdated(key);
                          setManualPasswordFor(null);
                        }}
                      />
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
        </div>
      )}

      {storeKeyResults.length > 0 && (
        <div className="found-keys-group">
          <p className="signing-section-label">스토어 키(.p8)</p>
          <p className="signing-key-meta">실제 업로드는 "스토어 자동 업로드" 기능에서 사용해요.</p>
          <ul className="signing-key-list">
            {storeKeyResults.map((found) => {
              const recorded = recordedPaths.has(found.path);
              return (
                <li key={found.path} className="signing-key-item">
                  <SigningKeyKindIcon kind="ios_api_key" />
                  <div className="signing-key-body">
                    <div className="signing-key-name-row">
                      <span className="signing-key-name">{P8_SUBTYPE_LABEL[found.kind.subtype]}</span>
                      <span className="signing-key-meta">Key ID {found.kind.keyId}</span>
                      {looksLikeCloudPath(found.path) && (
                        <span className="pill pill-info">☁ {cloudKindLabelFromPath(found.path)}</span>
                      )}
                    </div>
                    <p className="signing-key-meta">{found.path}</p>
                    <div className="signing-key-actions">
                      <button
                        type="button"
                        className="btn btn-outline"
                        disabled={recorded || recordingPath === found.path}
                        onClick={() => void handleRecordStoreKey(found)}
                      >
                        {recordingPath === found.path && <SpinnerIcon />}
                        {recorded ? "기록됨" : recordingPath === found.path ? "기록하는 중…" : "기록"}
                      </button>
                    </div>
                  </div>
                </li>
              );
            })}
          </ul>
        </div>
      )}

      {/* 클라우드 확인 Modal — 부모(SigningKeysSection)의 "내 컴퓨터에서 찾기" Modal 이 이미 열려 있는
          동안 그 위에 한 겹 더 뜬다(중첩). 이 패널이 이미 그 Modal 안에서만 마운트되므로(파일 상단
          문서 참고) 별도 리셋 없이 부모 Modal 을 닫으면 이 상태도 함께 사라진다. 다운로드 여부에 따라
          제목/본문/버튼만 갈라 보여준다 — 상태(cloudCheck) 하나로 두 화면을 감당한다(scanModalOpen /
          confirmRemove 와 같은 "한 상태 = 한 모달" 패턴). */}
      <Modal
        open={cloudCheck !== null}
        onClose={() => setCloudCheck(null)}
        title={cloudCheck?.info.isDownloaded ? "클라우드 서명키 복사" : "다운로드가 필요해요"}
      >
        {cloudCheck &&
          (cloudCheck.info.isDownloaded ? (
            <>
              <p className="confirm-remove-text">
                이 서명키는 {cloudCheck.info.cloudKind ?? "클라우드 저장소"}의{" "}
                <strong>{cloudCheck.info.folderName}</strong> 폴더에 있어요. 빌도락 금고로 복사해서 보관할게요
                (원본은 그대로 둡니다).
              </p>
              <div className="card-actions">
                <button type="button" className="btn btn-outline" onClick={() => setCloudCheck(null)}>
                  취소
                </button>
                <button type="button" className="btn btn-primary" onClick={handleConfirmCloudImport}>
                  복사하고 등록
                </button>
              </div>
            </>
          ) : (
            <>
              <p className="confirm-remove-text">
                ⚠️ 이 서명키(<strong>{cloudCheck.info.folderName}</strong>)가 아직 다운로드되지 않았어요.{" "}
                {cloudCheck.info.cloudKind ?? "클라우드 저장소"}에서 먼저 받아야 해요.
              </p>
              {actionError && (
                <div className="banner-error">
                  <CheckStatusIcon status="fail" />
                  <span>{actionError}</span>
                </div>
              )}
              <div className="card-actions">
                <button type="button" className="btn btn-outline" onClick={() => setCloudCheck(null)}>
                  닫기
                </button>
                <button
                  type="button"
                  className="btn btn-primary"
                  disabled={revealingPath === cloudCheck.found.path}
                  onClick={() => void handleRevealInFinder(cloudCheck.found.path)}
                >
                  {revealingPath === cloudCheck.found.path && <SpinnerIcon />}
                  Finder에서 열기
                </button>
              </div>
            </>
          ))}
      </Modal>
    </div>
  );
}

/** 이 서명키가 이 프로젝트 말고 다른 앱에도 연결돼 있으면 그 이름들 — "다른 앱에서도 사용 중" 표시용.
 * 체크리스트 완료 행(대표 서명키)과 "그 외 연결된 키" 목록이 같은 계산을 공유한다(중복 구현 금지). */
function otherLinkedAppNames(
  key: SigningKeyRecord,
  projectId: string,
  projectNamesById: Record<string, string>,
): string[] {
  return key.linkedProjectIds
    .filter((id) => id !== projectId)
    .map((id) => projectNamesById[id])
    .filter((name): name is string => Boolean(name));
}

export function SigningKeysSection({
  projectId,
  projectRootPath,
  allKeys,
  projectNamesById,
  onRegistered,
  onRemoved,
  onUpdated,
}: {
  projectId: string;
  /** 이 프로젝트의 루트 경로(ProjectRecord.selectedPath — 사용자가 실제로 고른 폴더) — "혹시 이건가?"
   * 하이라이트가 폴더 이름을 뽑는 데만 쓴다(copy.ts::projectFolderName). ProjectRecord.repoPath(pubspec.
   * yaml 이 있는 실제 빌드 루트, model.rs 문서 참고)는 pubspec 위치에 따라 selectedPath 하위의 다른
   * 폴더(예: "app")를 가리킬 수 있어 폴더 이름 매칭에 쓰면 안 된다 — repoPath 의 마지막 세그먼트가
   * "app" 같은 흔한 이름이 되면서 무관한 다른 프로젝트의 keystore 까지 매칭되는 버그가 있었다(리뷰
   * 지적, foundKeyMatchesProject 의 COMMON_PATH_TOKENS 하드닝도 같은 사고의 2차 방어선). 그 외 이
   * 컴포넌트에서 파일 경로를 직접 다루는 곳은 없다. */
  projectRootPath: string;
  /** 전역 서명키 목록(App.tsx 가 한 번 불러와 모든 카드에 그대로 내려준다). */
  allKeys: SigningKeyRecord[];
  /** 다른 프로젝트 이름 표시용("다른 앱에서도 사용 중") — App.tsx 가 projects 목록에서 한 번만 만든다. */
  projectNamesById: Record<string, string>;
  onRegistered: (key: SigningKeyRecord) => void;
  onRemoved: (keyId: string) => void;
  onUpdated: (key: SigningKeyRecord) => void;
}) {
  const { t } = useSettings();
  const [adding, setAdding] = useState(false);
  // register_signing_key 가 keystore 를 안전 보관 볼트로 복사하는 동안(signing.rs::
  // copy_keystore_into_vault) 원본이 클라우드(구글드라이브/iCloud 등) online-only 파일이면 백엔드가
  // 온디맨드 다운로드를 기다리며 최대 ~30초 재시도한다 — 그 시간 동안 화면이 멈춘 것처럼 보이지
  // 않도록 handleAddKey 가 일정 시간 뒤 이 플래그를 켠다(진행률을 아는 건 아니라 best-effort 힌트만).
  const [longWait, setLongWait] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // "서명키 추가" 직후 key.properties 자동 채움 성공을 알리는 안내 문구(FoundKeysPanel::successMessage
  // 와 같은 목적, 이 컴포넌트 스코프 전용) — 실패 시에는 아래 AndroidSigningForm 이 autoOpen 상태로
  // 직접 "비밀번호를 자동으로 못 찾았어요" 를 보여주므로(AndroidSigningForm 문서 참고) 여기서 중복
  // 문구를 띄우지 않는다(성공만 여기, 실패는 폼 인라인).
  const [notice, setNotice] = useState<string | null>(null);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [removingId, setRemovingId] = useState<string | null>(null);
  /** "빌도락에서 제거" 확인 모달 대상 — null 이면 모달이 닫혀 있다. 체크리스트(대표 서명키)와 "그 외
   * 연결된 키" 두 행의 [빌도락에서 제거] 버튼이 이 상태 하나를 공유한다(모달은 한 번에 하나만 뜨면
   * 되므로, scanModalOpen 과 같은 패턴). */
  const [confirmRemove, setConfirmRemove] = useState<SigningKeyRecord | null>(null);
  // 홑파일 keystore(옆에 key.properties 없음)를 "서명키 추가"로 등록했는데 이 프로젝트 자체의
  // key.properties 에서도 비밀번호를 못 찾았을 때만 채워진다(handleAddKey) — 체크리스트 "서명" 행의
  // AndroidSigningForm 을 자동으로 펼치고 alias 를 pre-fill 한다(FoundKeysPanel::manualPasswordFor 와
  // 같은 목적, 이 컴포넌트 스코프에서만 쓰는 별도 상태).
  const [manualPasswordFor, setManualPasswordFor] = useState<{ keyId: string; alias: string } | null>(null);

  // 체크리스트 "서명" 행 라벨용 — applicationId(우선)/namespace(폴백). 못 구해도(하드 에러 아님) 화면은
  // 앱 이름만 보여준다(App.tsx/ProjectCard 가 이미 표시 중이므로 여기는 보조 정보일 뿐이다).
  const [appId, setAppId] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    getProjectAppId(projectId)
      .then((id) => {
        if (!cancelled) setAppId(id);
      })
      .catch(() => {}); // 확인 못 해도 조용히 무시 — appId 는 없으면 그냥 안 보여줄 뿐인 보조 정보다.
    return () => {
      cancelled = true;
    };
  }, [projectId]);

  // "혹시 이건가?" 하이라이트(FoundKeysPanel)용 폴더 이름 — projectRootPath 는 props 로 이미 동기
  // 확보돼 있어(appId 처럼 Tauri 호출이 필요 없다) useEffect 없이 매 렌더 순수 계산한다.
  const projectFolder = projectFolderName(projectRootPath);

  // 체크리스트 "업로드" 행의 계정 수준 .p8 개수 — FoundKeysPanel 의 "기록" 액션이 성공하면
  // handleStoreKeyRecorded 로 갱신된다(단일 출처, FoundKeysPanel 자체 상태로 중복 들고 있지 않는다).
  const [foundStoreKeys, setFoundStoreKeys] = useState<FoundStoreKeyRecord[]>([]);
  useEffect(() => {
    listFoundStoreKeys()
      .then(setFoundStoreKeys)
      .catch(() => {}); // 실패는 조용히 무시 — 카운트만 안 보일 뿐 나머지 화면은 정상 동작한다.
  }, []);
  const handleStoreKeyRecorded = (record: FoundStoreKeyRecord) => {
    setFoundStoreKeys((prev) => (prev.some((r) => r.id === record.id) ? prev : [...prev, record]));
  };

  // "내 컴퓨터에서 찾기" 스캔 상태 — 체크리스트 "서명" 행의 CTA 버튼과 FoundKeysPanel 의 버튼이 같은
  // 결과를 공유해야 해서 여기(부모)로 끌어올렸다(버튼은 둘, 스캔 상태는 하나). 개별 후보 등록/기록
  // 액션의 상태는 여전히 FoundKeysPanel 이 스스로 관리한다(FoundKeysPanel 문서 참고).
  const [scanning, setScanning] = useState(false);
  const [scanned, setScanned] = useState(false);
  const [foundKeys, setFoundKeys] = useState<FoundKey[]>([]);
  const [scanError, setScanError] = useState<string | null>(null);
  const handleScan = async () => {
    if (scanning) return;
    setScanning(true);
    setScanError(null);
    try {
      const results = await scanSigningKeys();
      setFoundKeys(results);
      setScanned(true);
    } catch (e) {
      setScanError(typeof e === "string" ? e : "이 컴퓨터에서 서명키를 찾지 못했어요.");
    } finally {
      setScanning(false);
    }
  };

  // "내 컴퓨터에서 찾기" 결과를 담는 모달의 열림 상태 — 체크리스트 CTA 와 하단 트리거 버튼이 둘 다 이걸
  // 켠다. 열 때마다 스캔도 같이 시작한다(자동 진행, 아래 openScanModal) — FoundKeysPanel 은 모달이 열려
  // 있는 동안만 마운트되므로(Modal.tsx 문서 참고) 그 안의 로컬 상태(manualPasswordFor 등)는 열 때마다
  // 자연히 새로 시작한다.
  const [scanModalOpen, setScanModalOpen] = useState(false);
  const openScanModal = () => {
    setScanModalOpen(true);
    void handleScan();
  };

  const linkedKeys = allKeys.filter((key) => key.linkedProjectIds.includes(projectId));
  const availableKeys = allKeys.filter((key) => !key.linkedProjectIds.includes(projectId));
  // "대표 서명키" — 체크리스트 "서명" 행이 요약하는 단 하나의 Android keystore. 여러 개 연결돼 있으면
  // (드묾) 첫 번째만 요약하고, 나머지는 아래 "그 외 연결된 키"에서 계속 관리할 수 있다(정보 유실 없음).
  const androidKey = linkedKeys.find((key) => key.kind === "android_keystore") ?? null;
  const androidSigning = androidKey?.androidSigning ?? null;
  const androidCertMeta = androidSigning ? androidCertMetaLine(androidSigning) : null;
  // "그 외 연결된 키" 목록과 같은 계산(otherLinkedAppNames) — 체크리스트 "서명" 행에서도 "다른 앱에서도
  // 사용 중" 표시가 사라지지 않게 한다.
  const androidOtherApps = androidKey ? otherLinkedAppNames(androidKey, projectId, projectNamesById) : [];
  // 안전 보관(볼트 복사) 표시 — vaultPath 가 있을 때만(이 기능 이전 레코드는 없음, copy.ts::
  // vaultStatusLine 문서 참고).
  const androidVaultLine = androidKey ? vaultStatusLine(androidKey) : null;
  // handleAddKey 가 자동 채움에 실패했을 때만 채워진다(manualPasswordFor) — 이 대표 서명키 자리에 해당할
  // 때만 폼을 자동으로 펼치고 alias 를 pre-fill 한다(FoundKeysPanel 의 manualEntry 판정과 같은 패턴).
  const checklistManualEntry =
    manualPasswordFor && manualPasswordFor.keyId === androidKey?.id ? manualPasswordFor : null;
  // 체크리스트에 이미 요약된 대표 서명키를 뺀 나머지 — 다른 종류(iOS 인증서 등)이거나 추가로 연결된
  // Android keystore. 하나도 없으면(가장 흔한 케이스: Android keystore 하나만 연결) 이 목록 자체를
  // 렌더링하지 않는다(체크리스트와 중복 표시하지 않기 위해).
  const otherLinkedKeys = linkedKeys.filter((key) => key.id !== androidKey?.id);

  const handleAddKey = async () => {
    if (adding) return;
    setAdding(true);
    setError(null);
    setNotice(null);
    // 파일 선택 다이얼로그(사용자가 얼마나 오래 붙잡고 있을지 알 수 없다)가 끝난 "다음", 실제 볼트
    // 복사가 시작될 때만 타이머를 건다 — 클라우드 온디맨드 다운로드로 오래 걸려도(백엔드 재시도 상한
    // ~30초) 사용자가 "멈췄나?" 오해하지 않게 문구만 바꾼다(실제 진행 상황과 무관한 고정 지연 힌트).
    let longWaitTimer: ReturnType<typeof setTimeout> | undefined;
    try {
      const fileToken = await pickSigningKeyFile();
      if (!fileToken) return; // 사용자가 다이얼로그를 취소함
      longWaitTimer = setTimeout(() => setLongWait(true), 4000);
      const registered = await registerSigningKey(fileToken);
      onRegistered(registered);
      // 이 카드에서 추가했으니 바로 이 프로젝트에 연결한다 — 연결이 실패해도(드묾) 등록 자체는 이미
      // 반영됐으니 "등록된 다른 서명키 연결" 목록에서 다시 시도할 수 있다(데이터 유실 없음).
      const linked = await linkSigningKey(registered.id, projectId);
      onUpdated(linked);

      // 홑파일 keystore(옆에 key.properties 없음)면 이 프로젝트 자체의 key.properties 에서 비밀번호
      // 자동 채움을 시도한다(확정된 설계 결정) — Android keystore 이고 아직 비밀번호가 없을 때만.
      // storeFile 이 이 keystore 로 정확히 resolve 될 때만 채워지고(안전 매칭), 불일치/파일없음이면
      // imported:false 로 와서 아래 체크리스트의 AndroidSigningForm 을 자동으로 펼쳐 수동 입력을 받는다.
      if (linked.kind === "android_keystore" && !linked.androidSigning) {
        try {
          const result = await autofillAndroidSigning(linked.id, projectId);
          onUpdated(result.key);
          if (result.imported) {
            setNotice(
              `${result.key.displayName} — ✓ 등록됨 · key.properties 에서 비밀번호를 자동으로 찾아 연결했어요.`,
            );
          } else {
            setManualPasswordFor({ keyId: linked.id, alias: result.keyAlias ?? "" });
          }
        } catch {
          // 자동 채움 시도 자체가 실패해도(드묾) 등록·연결은 이미 끝났다 — 조용히 수동 입력으로 넘어간다.
          setManualPasswordFor({ keyId: linked.id, alias: "" });
        }
      }
    } catch (e) {
      setError(typeof e === "string" ? e : "서명키를 등록하지 못했어요. 잠시 후 다시 시도해 주세요.");
    } finally {
      if (longWaitTimer) clearTimeout(longWaitTimer);
      setLongWait(false);
      setAdding(false);
    }
  };

  const handleLink = async (keyId: string) => {
    if (pendingId) return;
    setPendingId(keyId);
    setError(null);
    try {
      const updated = await linkSigningKey(keyId, projectId);
      onUpdated(updated);
    } catch (e) {
      setError(typeof e === "string" ? e : "서명키를 연결하지 못했어요.");
    } finally {
      setPendingId(null);
    }
  };

  const handleUnlink = async (keyId: string) => {
    if (pendingId) return;
    setPendingId(keyId);
    setError(null);
    try {
      const updated = await unlinkSigningKey(keyId, projectId);
      onUpdated(updated);
    } catch (e) {
      setError(typeof e === "string" ? e : "서명키 연결을 해제하지 못했어요.");
    } finally {
      setPendingId(null);
    }
  };

  /**
   * "빌도락에서 제거"(구 "완전히 삭제") 확인 — window.confirm 대신 이 컴포넌트가 이미 쓰는 Modal 을
   * 재사용한다(Tauri 환경에서 window.confirm 이 불안정, 리뷰 지적). confirmRemove 에 대상 키를 담아
   * 모달을 열고, 실제 삭제는 모달 안 [빌도락에서 제거] 버튼을 눌러야 실행된다. 실패하면 모달을 닫지
   * 않고 안에 에러를 보여준다(재시도/취소 모두 그 자리에서 가능하도록) — 성공했을 때만 confirmRemove
   * 를 비워 모달을 닫는다.
   */
  const handleRemove = async (key: SigningKeyRecord) => {
    if (removingId) return;
    setRemovingId(key.id);
    setError(null);
    try {
      await removeSigningKey(key.id);
      onRemoved(key.id);
      setConfirmRemove(null);
    } catch (e) {
      setError(typeof e === "string" ? e : "서명키를 빌도락에서 제거하지 못했어요.");
    } finally {
      setRemovingId(null);
    }
  };

  return (
    <div className="signing-section">
      <div className="signing-section-label">{t("signing.checklistTitle")}</div>
      {appId && <p className="signing-key-meta">{appId}</p>}

      {error && (
        <div className="banner-error">
          <CheckStatusIcon status="fail" />
          <span>{error}</span>
        </div>
      )}
      {notice && <p className="banner-success">{notice}</p>}

      <ul className="signing-key-list">
        <li className="signing-key-item">
          <SigningKeyKindIcon kind="android_keystore" />
          <div className="signing-key-body">
            <div className="signing-key-name-row">
              <span className="signing-key-name">{t("signing.signRow")}</span>
              {androidKey && signingKeyExpiryStatus(androidKey) !== "unknown" && (
                <span className={`pill ${EXPIRY_PILL_CLASS[signingKeyExpiryStatus(androidKey)]}`}>
                  {signingKeyExpiryLabel(androidKey)}
                </span>
              )}
              <span className={`pill ${androidSigning ? "pill-ok" : "pill-idle"}`}>
                {androidSigning ? "✓ 등록됨" : "○ 아직"}
              </span>
            </div>
            <p className="signing-key-meta">
              {androidSigning
                ? `${androidSigning.keyAlias}${androidCertMeta ? ` · ${androidCertMeta}` : ""}`
                : androidKey
                  ? `${androidKey.displayName} · 비밀번호 등록이 필요해요.`
                  : "아직 등록된 서명키가 없어요."}
            </p>
            {androidVaultLine && <p className="signing-key-meta">{androidVaultLine}</p>}
            {androidOtherApps.length > 0 && (
              <p className="signing-key-linked-apps">다른 앱에서도 사용 중: {androidOtherApps.join(", ")}</p>
            )}
            <div className="signing-key-actions">
              {androidKey ? (
                <>
                  <AndroidSigningForm
                    keyId={androidKey.id}
                    currentAlias={checklistManualEntry ? checklistManualEntry.alias : androidSigning?.keyAlias}
                    autoOpen={Boolean(checklistManualEntry)}
                    onSaved={(key) => {
                      onUpdated(key);
                      setManualPasswordFor(null);
                    }}
                  />
                  <button
                    type="button"
                    className="btn-text-secondary"
                    disabled={pendingId === androidKey.id}
                    onClick={() => void handleUnlink(androidKey.id)}
                  >
                    {pendingId === androidKey.id ? "해제하는 중…" : "이 앱에서 연결 해제"}
                  </button>
                  <span className="signing-key-actions-danger">
                    <button
                      type="button"
                      className="btn-danger-text"
                      onClick={() => {
                        setError(null);
                        setConfirmRemove(androidKey);
                      }}
                    >
                      빌도락에서 제거
                    </button>
                  </span>
                </>
              ) : (
                <button type="button" className="btn btn-outline" disabled={scanning} onClick={openScanModal}>
                  {scanning && <SpinnerIcon />}
                  {scanning ? "찾는 중…" : "내 컴퓨터에서 찾기 · 등록"}
                </button>
              )}
            </div>
          </div>
        </li>

        <li className="signing-key-item">
          <SigningKeyKindIcon kind="ios_api_key" />
          <div className="signing-key-body">
            <div className="signing-key-name-row">
              <span className="signing-key-name">{t("signing.uploadRow")}</span>
              <span className="pill pill-idle">○ 아직</span>
            </div>
            <p className="signing-key-meta">
              {foundStoreKeys.length > 0
                ? `애플 출입증 .p8 ${foundStoreKeys.length}개 발견됨 · 자동 업로드 곧 지원`
                : "아직 발견된 애플 출입증(.p8)이 없어요 · 자동 업로드 곧 지원"}
            </p>
          </div>
        </li>
      </ul>

      {otherLinkedKeys.length > 0 && (
        <div className="signing-available-list">
          <p className="signing-section-label">그 외 연결된 키</p>
          <ul className="signing-key-list">
            {otherLinkedKeys.map((key) => {
              const otherApps = otherLinkedAppNames(key, projectId, projectNamesById);
              const certMeta =
                key.kind === "android_keystore" && key.androidSigning
                  ? androidCertMetaLine(key.androidSigning)
                  : null;
              const vaultLine = key.kind === "android_keystore" ? vaultStatusLine(key) : null;
              return (
                <li key={key.id} className="signing-key-item">
                  <SigningKeyKindIcon kind={key.kind} />
                  <div className="signing-key-body">
                    <div className="signing-key-name-row">
                      <span className="signing-key-name">{key.displayName}</span>
                      <span className={`pill ${EXPIRY_PILL_CLASS[signingKeyExpiryStatus(key)]}`}>
                        {signingKeyExpiryLabel(key)}
                      </span>
                      {key.kind === "android_keystore" && (
                        <span className={`pill ${key.androidSigning ? "pill-ok" : "pill-idle"}`}>
                          {key.androidSigning ? "release 자동 서명 설정됨" : "release 자동 서명 미설정"}
                        </span>
                      )}
                    </div>
                    <p className="signing-key-meta">{SIGNING_KEY_KIND_LABEL[key.kind]}</p>
                    {certMeta && <p className="signing-key-meta">{certMeta}</p>}
                    {vaultLine && <p className="signing-key-meta">{vaultLine}</p>}
                    {otherApps.length > 0 && (
                      <p className="signing-key-linked-apps">다른 앱에서도 사용 중: {otherApps.join(", ")}</p>
                    )}
                    <div className="signing-key-actions">
                      {key.kind === "android_keystore" && (
                        <AndroidSigningForm keyId={key.id} currentAlias={key.androidSigning?.keyAlias} onSaved={onUpdated} />
                      )}
                      <button
                        type="button"
                        className="btn-text-secondary"
                        disabled={pendingId === key.id}
                        onClick={() => void handleUnlink(key.id)}
                      >
                        {pendingId === key.id ? "해제하는 중…" : "이 앱에서 연결 해제"}
                      </button>
                      <span className="signing-key-actions-danger">
                        <button
                          type="button"
                          className="btn-danger-text"
                          onClick={() => {
                            setError(null);
                            setConfirmRemove(key);
                          }}
                        >
                          빌도락에서 제거
                        </button>
                      </span>
                    </div>
                  </div>
                </li>
              );
            })}
          </ul>
        </div>
      )}

      <div className="card-actions">
        <button type="button" className="btn btn-outline" disabled={adding} onClick={() => void handleAddKey()}>
          {adding && <SpinnerIcon />}
          {adding ? (longWait ? "클라우드 파일 다운로드 중…" : "등록하는 중…") : "서명키 추가"}
        </button>
        <button type="button" className="btn btn-outline" disabled={scanning} onClick={openScanModal}>
          {scanning && <SpinnerIcon />}
          {scanning ? "찾는 중…" : "내 컴퓨터에서 찾기"}
        </button>
      </div>

      <Modal open={scanModalOpen} onClose={() => setScanModalOpen(false)} title="내 컴퓨터에서 찾기">
        <FoundKeysPanel
          projectId={projectId}
          allKeys={allKeys}
          onRegistered={onRegistered}
          onUpdated={onUpdated}
          scanning={scanning}
          scanned={scanned}
          foundKeys={foundKeys}
          scanError={scanError}
          onScan={() => void handleScan()}
          recommendedAppId={appId}
          recommendedFolderName={projectFolder}
          foundStoreKeys={foundStoreKeys}
          onStoreKeyRecorded={handleStoreKeyRecorded}
        />
      </Modal>

      <Modal open={confirmRemove !== null} onClose={() => setConfirmRemove(null)} title="빌도락에서 제거">
        {confirmRemove && (
          <>
            <p className="confirm-remove-text">
              <strong>{confirmRemove.displayName}</strong> 서명키를 빌도락에서 제거할까요?
              {(() => {
                const others = otherLinkedAppNames(confirmRemove, projectId, projectNamesById);
                return others.length > 0 ? ` 연결된 다른 앱(${others.join(", ")})에서도 함께 사라져요.` : "";
              })()}
            </p>
            <p className="confirm-remove-text">
              빌도락 등록과 저장된 비밀번호를 지웁니다.{" "}
              <strong>원본 keystore 파일과 프로젝트 설정은 그대로예요.</strong>
            </p>
            {error && (
              <div className="banner-error">
                <CheckStatusIcon status="fail" />
                <span>{error}</span>
              </div>
            )}
            <div className="card-actions">
              <button
                type="button"
                className="btn btn-outline"
                disabled={removingId === confirmRemove.id}
                onClick={() => {
                  setConfirmRemove(null);
                  setError(null);
                }}
              >
                취소
              </button>
              <button
                type="button"
                className="btn btn-danger"
                disabled={removingId === confirmRemove.id}
                onClick={() => void handleRemove(confirmRemove)}
              >
                {removingId === confirmRemove.id && <SpinnerIcon />}
                {removingId === confirmRemove.id ? "제거하는 중…" : "빌도락에서 제거"}
              </button>
            </div>
          </>
        )}
      </Modal>

      {availableKeys.length > 0 && (
        <div className="signing-available-list">
          <p className="signing-section-label">등록된 다른 서명키 연결</p>
          <ul className="signing-key-list">
            {availableKeys.map((key) => (
              <li key={key.id} className="signing-key-item">
                <SigningKeyKindIcon kind={key.kind} />
                <div className="signing-key-body">
                  <div className="signing-key-name-row">
                    <span className="signing-key-name">{key.displayName}</span>
                    <span className={`pill ${EXPIRY_PILL_CLASS[signingKeyExpiryStatus(key)]}`}>
                      {signingKeyExpiryLabel(key)}
                    </span>
                  </div>
                  <p className="signing-key-meta">{SIGNING_KEY_KIND_LABEL[key.kind]}</p>
                  <div className="signing-key-actions">
                    <button
                      type="button"
                      className="btn btn-outline"
                      disabled={pendingId === key.id}
                      onClick={() => void handleLink(key.id)}
                    >
                      {pendingId === key.id ? "연결하는 중…" : "이 앱에 연결"}
                    </button>
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
