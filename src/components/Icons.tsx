// Icons.tsx — 인라인 SVG 아이콘(이모지 금지 원칙).

import type { CheckStatus, SigningKeyKind } from "../lib/types";

export function SpinnerIcon() {
  return (
    <svg
      width={14}
      height={14}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      aria-hidden="true"
      focusable="false"
      className="spinner"
    >
      <path d="M21 12a9 9 0 1 1-6.219-8.56" />
    </svg>
  );
}

/** 점검 항목 하나의 상태 아이콘 — 통과(체크)/주의·실패(경고 삼각형). */
export function CheckStatusIcon({ status }: { status: CheckStatus }) {
  if (status === "pass") {
    return (
      <svg
        width={16}
        height={16}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={2}
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
        focusable="false"
        className="check-icon ok"
      >
        <path d="M20 6 9 17l-5-5" />
      </svg>
    );
  }
  return (
    <svg
      width={16}
      height={16}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      className={`check-icon ${status === "fail" ? "fail" : "warn"}`}
    >
      <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
      <path d="M12 9v4" />
      <path d="M12 17h.01" />
    </svg>
  );
}

/** 모달 닫기(X) — Modal.tsx 전용. 이모지 금지 원칙 그대로 인라인 SVG. */
export function CloseIcon() {
  return (
    <svg
      width={16}
      height={16}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
    </svg>
  );
}

/** 서명키 종류 아이콘 — 이모지 대신 인라인 SVG(파일 상단 "이모지 금지 원칙" 그대로 유지). 브랜드
 * 로고(사과 등) 대신 의미가 통하는 일반 도형(인증서/열쇠/업로드)을 쓴다. */
export function SigningKeyKindIcon({ kind }: { kind: SigningKeyKind }) {
  if (kind === "ios_cert") {
    // 인증서/보증 배지 — 방패 + 체크.
    return (
      <svg
        width={16}
        height={16}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={2}
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
        focusable="false"
        className="signing-key-icon"
      >
        <path d="M12 3 4 6v6c0 5 3.5 9.5 8 10 4.5-.5 8-5 8-10V6Z" />
        <path d="m9 12 2 2 4-4" />
      </svg>
    );
  }
  if (kind === "android_keystore") {
    // 열쇠 — keystore 은유.
    return (
      <svg
        width={16}
        height={16}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={2}
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
        focusable="false"
        className="signing-key-icon"
      >
        <circle cx="8" cy="15" r="4" />
        <path d="m11 12 9-9" />
        <path d="m17 6 3 3" />
        <path d="m14 9 2 2" />
      </svg>
    );
  }
  // ios_api_key — App Store Connect 업로드/전송 은유.
  return (
    <svg
      width={16}
      height={16}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      className="signing-key-icon"
    >
      <path d="M12 3v12" />
      <path d="m7 8 5-5 5 5" />
      <path d="M5 21h14" />
    </svg>
  );
}
