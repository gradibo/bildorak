// Modal.tsx — 접근성 갖춘 범용 모달(포털 렌더). 이 프로젝트엔 기존 다이얼로그 라이브러리가 없어서
// (package.json 확인, radix/headlessui 등 미설치) 최소 구현을 새로 둔다. "내 컴퓨터에서 찾기" 결과와
// 서명키 "빌도락에서 제거" 확인을, 화면 아래로 스크롤하거나 불안정한 네이티브 대화상자 없이 그 자리에서
// 보여주기 위해 SigningKeysSection.tsx 가 쓴다 — open/onClose/title/children 만 있으면 되는 범용
// 컴포넌트라 다른 화면에서도 그대로 재사용할 수 있다.
//
// 네이티브 confirm 은 이제 이 프로젝트에서 쓰지 않는다 — Tauri 환경에서 window.confirm 이 불안정해
// (리뷰 지적) 서명키 "빌도락에서 제거" 확인도 이 컴포넌트로 옮겼다(SigningKeysSection.tsx::
// confirmRemove 참고). alert/prompt 대체 용도는 여전히 범위 밖.
//
// 접근성: role="dialog" + aria-modal, Escape/backdrop 클릭으로 닫기, 열릴 때 포커스 이동 + Tab 트랩,
// 닫힐 때 이전 포커스로 복귀, 열려 있는 동안 body 스크롤 락. prefers-reduced-motion 은 App.css 의
// 애니메이션 쪽에서 처리(이 파일은 로직만).
//
// open=false 일 땐 아예 렌더하지 않는다(children 도 마운트 안 됨) — 그래서 이 모달을 여닫을 때마다
// children 쪽 로컬 상태(예: FoundKeysPanel::manualPasswordFor)가 저절로 초기화된다. 별도 리셋 코드가
// 필요 없다.
import { useEffect, useId, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { CloseIcon } from "./Icons";

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

// 모듈 레벨 모달 스택 - 중첩된 Modal(릴리스 삭제확인이 편집 폼 위에, 서명키 클라우드 확인이 스캔 결과
// 모달 위에 뜨는 것처럼)이 동시에 열려 있으면, 각 인스턴스가 document 에 따로 붙인 keydown 리스너가
// 전부 같은 Escape 이벤트를 받아 한 번에 다 닫혀버리는 문제가 있었다(리뷰 지적 - stopPropagation 은
// 같은 노드에 붙은 다른 리스너를 막지 못한다). 열리는 Modal 마다 이 배열에 자기 id 를 push, 닫히거나
// unmount 될 때 pop 해서 "지금 스택 맨 위(가장 나중에 열린 모달)가 누구인지" 를 전역으로 추적한다 -
// Escape 핸들러는 자기가 맨 위일 때만 반응한다. 컴포넌트 상태가 아니라 모듈 변수로 둔 이유는 서로
// 다른 Modal 인스턴스(=서로 다른 React 트리 위치) 사이에서 공유돼야 하는 값이라 각 인스턴스의 로컬
// state/ref 로는 표현할 수 없어서다. body 스크롤 락도 이 스택을 같이 써서, 스택이 완전히 빌 때만
// 원래 overflow 값으로 복원한다(중첩 상태에서 안쪽 모달만 먼저 닫혀도 바깥 모달이 열려 있는 동안은
// 스크롤이 계속 잠겨 있어야 한다).
let modalStack: number[] = [];
let modalIdCounter = 0;
let bodyOverflowBeforeStack = "";

export function Modal({
  open,
  onClose,
  title,
  children,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
}) {
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const previouslyFocusedRef = useRef<HTMLElement | null>(null);
  // onClose 는 ref 로 최신 값만 참조한다 — 호출부가 매 렌더 새 화살표 함수를 넘겨도(onClose={() =>
  // setOpen(false)} 같은 흔한 인라인 패턴) 아래 effect 가 그때마다 재실행되며 포커스 복귀·스크롤 락을
  // 반복하지 않게 하기 위함이다. effect 의 deps 는 [open] 하나뿐이다.
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    if (!open) return;

    previouslyFocusedRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;

    // 스택이 비어 있을 때(=지금 열리는 내가 첫 모달)만 "원래" overflow 값을 기록한다 - 중첩된 두 번째
    // 이후 모달은 이미 hidden 인 값을 기록해 버리면 나중에 잘못된 값으로 복원하게 된다(파일 상단 문서
    // 참고).
    if (modalStack.length === 0) {
      bodyOverflowBeforeStack = document.body.style.overflow;
    }
    modalIdCounter += 1;
    const myModalId = modalIdCounter;
    modalStack.push(myModalId);
    document.body.style.overflow = "hidden";

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // 스택 맨 위(가장 나중에 열린 모달)가 아니면 무시 - 중첩된 아래쪽 모달은 이 Escape 에 반응하지
        // 않는다(파일 상단 문서 참고, 리뷰 지적).
        if (modalStack[modalStack.length - 1] !== myModalId) return;
        e.stopPropagation();
        onCloseRef.current();
        return;
      }
      if (e.key !== "Tab") return;
      const container = dialogRef.current;
      if (!container) return;
      const focusable = Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
      if (focusable.length === 0) {
        e.preventDefault();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", handleKeyDown);

    // 다음 tick 에 포커스 — dialog 가 막 DOM 에 그려진 직후라야 focus() 가 먹는다(AndroidSigningForm 의
    // autoOpen 포커스 패턴과 같은 이유, SigningKeysSection.tsx 참고).
    const focusTimer = window.setTimeout(() => {
      const container = dialogRef.current;
      const target = container?.querySelector<HTMLElement>(FOCUSABLE_SELECTOR) ?? container;
      target?.focus();
    }, 0);

    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      window.clearTimeout(focusTimer);
      modalStack = modalStack.filter((id) => id !== myModalId);
      // 스택이 완전히 비었을 때(=마지막으로 열려 있던 모달까지 닫힘)만 원래 overflow 값으로 복원한다 -
      // 중첩 상태에서 안쪽 모달이 먼저 닫혀도 바깥 모달이 아직 열려 있으면 스크롤 락을 유지한다.
      if (modalStack.length === 0) {
        document.body.style.overflow = bodyOverflowBeforeStack;
      }
      previouslyFocusedRef.current?.focus();
    };
  }, [open]);

  if (!open) return null;

  return createPortal(
    <div
      className="modal-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        className="modal-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        ref={dialogRef}
        tabIndex={-1}
      >
        <div className="modal-header">
          <h2 id={titleId} className="modal-title">
            {title}
          </h2>
          <button type="button" className="modal-close" aria-label="닫기" onClick={onClose}>
            <CloseIcon />
          </button>
        </div>
        <div className="modal-body">{children}</div>
      </div>
    </div>,
    document.body,
  );
}
