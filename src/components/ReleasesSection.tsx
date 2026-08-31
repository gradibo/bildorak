// ReleasesSection.tsx - ProjectCard 안, SigningKeysSection 아래에 붙는 "릴리스" 섹션(릴리스 관리 1차
// 슬라이스). 이 앱이 지금까지 어떤 버전을 어느 채널에 언제 냈고 지금 어떤 상태인지 타임라인으로
// 보여준다. 빌드 히스토리(ProjectCard.tsx::historyOpen)와 동일하게 기본은 접힘 + 펼칠 때만 list_releases
// 를 불러온다 - 프로젝트마다 매번 조회하지 않는다.
//
// 행 클릭(또는 "새 릴리스" 버튼) → Modal(범용, Modal.tsx) 재사용 폼에서 버전/빌드번호/채널/상태/노트를
// 입력받는다. "새 릴리스" 는 get_project_current_version 으로 pubspec.yaml 을 지금 다시 읽어 버전을
// pre-fill 하고, 실패하면(폴더가 옮겨졌거나 등) project.version 스냅샷으로 조용히 폴백한다(에러 배너를
// 띄우지 않는다 - ProjectCard.tsx::fetchBuildStatus 의 무음 폴백과 같은 원칙).
//
// 삭제는 SigningKeysSection::confirmRemove 와 동일하게 별도 Modal 확인을 거친다(Tauri 환경 window.confirm
// 불안정, 그 파일 상단 문서 참고) - 편집 폼 안의 [삭제] 버튼이 이 확인 모달을 연다.
//
// 빌드 이력 연결·GitHub 연동·제출 자동화·다중 스토어 상태·구조화 노트는 범위 밖(다음 로드맵 단계) -
// 지금은 순수 수동 기록이다.

import { useEffect, useRef, useState } from "react";
import { createRelease, deleteRelease, getProjectCurrentVersion, listReleases, updateRelease } from "../lib/api";
import { formatKst } from "../lib/copy";
import {
  RELEASE_CHANNEL_LABEL,
  RELEASE_STATUS_LABEL,
  type ReleaseChannel,
  type ReleaseRecord,
  type ReleaseStatus,
} from "../lib/types";
import { useSettings } from "../lib/settings-context";
import { CheckStatusIcon, SpinnerIcon } from "./Icons";
import { Modal } from "./Modal";

const RELEASE_CHANNEL_OPTIONS: ReleaseChannel[] = ["app_store", "play_store", "github", "other"];
const RELEASE_STATUS_OPTIONS: ReleaseStatus[] = ["preparing", "submitted", "approved", "rejected", "released"];

/** 상태 pill 색 - 기존 pill-ok/warn/crit/info/idle 5종을 그대로 재사용한다(새 색 추가 없음). */
const RELEASE_STATUS_PILL_CLASS: Record<ReleaseStatus, string> = {
  preparing: "pill-idle",
  submitted: "pill-info",
  approved: "pill-ok",
  rejected: "pill-crit",
  released: "pill-ok",
};

export function ReleasesSection({
  projectId,
  projectVersionSnapshot,
  projectBuildNumberSnapshot,
}: {
  projectId: string;
  /** 프로젝트 등록 시점 스냅샷(ProjectRecord.version) - "새 릴리스" 폼을 열 때 우선 이 값으로 채우고,
   * get_project_current_version 이 성공하면 지금 값으로 덮어쓴다(무음 폴백, 파일 상단 문서 참고). */
  projectVersionSnapshot: string | null;
  projectBuildNumberSnapshot: string | null;
}) {
  const { t } = useSettings();

  const [open, setOpen] = useState(false);
  const [releases, setReleases] = useState<ReleaseRecord[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);

  // "릴리스" 섹션을 열 때마다 다시 불러온다 - ProjectCard.tsx::historyOpen effect 와 동일 패턴.
  useEffect(() => {
    if (!open) return;
    setLoading(true);
    setListError(null);
    listReleases(projectId)
      .then(setReleases)
      .catch((e) => setListError(typeof e === "string" ? e : "릴리스 기록을 불러오지 못했어요."))
      .finally(() => setLoading(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, projectId]);

  // 새 릴리스/수정 폼(하나의 Modal 을 두 목적에 재사용) - editing 이 null 이면 "새 릴리스", 레코드가
  // 있으면 그 레코드를 고치는 중이다.
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<ReleaseRecord | null>(null);
  const [formVersion, setFormVersion] = useState("");
  const [formBuildNumber, setFormBuildNumber] = useState("");
  const [formChannel, setFormChannel] = useState<ReleaseChannel>("app_store");
  const [formStatus, setFormStatus] = useState<ReleaseStatus>("preparing");
  const [formNotes, setFormNotes] = useState("");
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [copiedNotes, setCopiedNotes] = useState(false);

  // getProjectCurrentVersion 응답이 늦게 도착했을 때, 그 사이 폼이 닫혔거나 다른 레코드 편집으로
  // 전환됐으면 그 폼을 덮어쓰지 않기 위한 세션 카운터 - handleOpenCreate/handleOpenEdit/closeForm 이
  // 폼의 "무엇을 보여줄지"를 바꿀 때마다 값을 새로 발급한다. 비동기 응답이 도착한 시점에 이 값이
  // 요청 당시 캡처해 둔 값과 다르면(그 사이 폼이 다시 열렸거나 닫혔다는 뜻) setState 를 건너뛴다.
  const formSessionRef = useRef(0);

  const closeForm = () => {
    formSessionRef.current += 1;
    setFormOpen(false);
  };

  const handleOpenCreate = () => {
    const session = ++formSessionRef.current;
    setEditing(null);
    setFormVersion(projectVersionSnapshot ?? "");
    setFormBuildNumber(projectBuildNumberSnapshot ?? "");
    setFormChannel("app_store");
    setFormStatus("preparing");
    setFormNotes("");
    setFormError(null);
    setCopiedNotes(false);
    setFormOpen(true);
    // pubspec.yaml 을 지금 다시 읽어 더 최신 값이 있으면 덮어쓴다 - 실패해도(폴더 이동 등) 위에서 이미
    // 채운 스냅샷 값을 그대로 둔다(무음 폴백).
    getProjectCurrentVersion(projectId)
      .then((current) => {
        if (formSessionRef.current !== session) return; // 그 사이 폼이 닫혔거나 다른 레코드로 전환됨
        if (current.version) setFormVersion(current.version);
        if (current.buildNumber) setFormBuildNumber(current.buildNumber);
      })
      .catch(() => {});
  };

  const handleOpenEdit = (release: ReleaseRecord) => {
    formSessionRef.current += 1; // 진행 중이던 "새 릴리스" pre-fill 요청을 무효화한다
    setEditing(release);
    setFormVersion(release.version);
    setFormBuildNumber(release.buildNumber ?? "");
    setFormChannel(release.channel);
    setFormStatus(release.status);
    setFormNotes(release.notes);
    setFormError(null);
    setCopiedNotes(false);
    setFormOpen(true);
  };

  const handleSave = async () => {
    if (saving || !formVersion.trim()) return;
    setSaving(true);
    setFormError(null);
    try {
      const buildNumber = formBuildNumber.trim() ? formBuildNumber.trim() : null;
      if (editing) {
        const updated = await updateRelease(editing.id, formVersion.trim(), buildNumber, formChannel, formStatus, formNotes);
        setReleases((prev) => (prev ? prev.map((r) => (r.id === updated.id ? updated : r)) : prev));
      } else {
        const created = await createRelease(projectId, formVersion.trim(), buildNumber, formChannel, formStatus, formNotes);
        setReleases((prev) => (prev ? [created, ...prev] : [created]));
      }
      closeForm();
    } catch (e) {
      setFormError(typeof e === "string" ? e : "릴리스를 저장하지 못했어요.");
    } finally {
      setSaving(false);
    }
  };

  const handleCopyNotes = () => {
    if (!editing || !editing.notes) return;
    navigator.clipboard
      .writeText(editing.notes)
      .then(() => {
        setCopiedNotes(true);
        setTimeout(() => setCopiedNotes(false), 1500);
      })
      .catch(() => {});
  };

  // 삭제 확인 - SigningKeysSection::confirmRemove 와 동일한 "한 상태 = 한 모달" 패턴. 편집 폼 안의
  // [삭제] 버튼이 이 상태를 채운다.
  const [confirmDelete, setConfirmDelete] = useState<ReleaseRecord | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const handleDelete = async (release: ReleaseRecord) => {
    if (deleting) return;
    setDeleting(true);
    setDeleteError(null);
    try {
      await deleteRelease(release.id);
      setReleases((prev) => (prev ? prev.filter((r) => r.id !== release.id) : prev));
      setConfirmDelete(null);
      closeForm();
    } catch (e) {
      setDeleteError(typeof e === "string" ? e : "릴리스를 삭제하지 못했어요.");
    } finally {
      setDeleting(false);
    }
  };

  return (
    <div className="releases-section">
      <div className="releases-section-label">{t("releases.sectionTitle")}</div>
      <button type="button" className="history-toggle" onClick={() => setOpen((v) => !v)}>
        {open ? "릴리스 접기" : "릴리스 보기"}
      </button>

      {open && (
        <div className="history-list">
          <div className="card-actions">
            <button type="button" className="btn btn-outline" onClick={handleOpenCreate}>
              새 릴리스
            </button>
          </div>

          {loading && <p className="history-empty">불러오는 중…</p>}
          {listError && (
            <div className="banner-error">
              <CheckStatusIcon status="fail" />
              <span>{listError}</span>
            </div>
          )}
          {!loading && !listError && releases && releases.length === 0 && (
            <p className="history-empty">아직 릴리스 기록이 없어요.</p>
          )}
          {!loading && releases && releases.length > 0 && (
            <ul className="history-items">
              {releases.map((release) => (
                <li key={release.id}>
                  <button
                    type="button"
                    className="history-item release-row-button"
                    onClick={() => handleOpenEdit(release)}
                  >
                    <span className={`pill ${RELEASE_STATUS_PILL_CLASS[release.status]}`}>
                      {RELEASE_STATUS_LABEL[release.status]}
                    </span>
                    <span className="history-item-target">
                      {release.version}
                      {release.buildNumber ? ` (빌드 ${release.buildNumber})` : ""}
                    </span>
                    <span className="signing-key-meta">{RELEASE_CHANNEL_LABEL[release.channel]}</span>
                    <span className="history-item-time">{formatKst(release.createdAt)}</span>
                    {release.notes && <span className="history-item-duration">메모 있음</span>}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      <Modal open={formOpen} onClose={closeForm} title={editing ? "릴리스 수정" : "새 릴리스"}>
        <label className="release-form-field">
          버전
          <input
            type="text"
            value={formVersion}
            onChange={(e) => setFormVersion(e.target.value)}
            placeholder="예: 1.2.0"
            autoComplete="off"
          />
        </label>
        <label className="release-form-field">
          빌드 번호(선택)
          <input
            type="text"
            value={formBuildNumber}
            onChange={(e) => setFormBuildNumber(e.target.value)}
            placeholder="예: 42"
            autoComplete="off"
          />
        </label>
        <div className="release-form-row">
          <label className="release-form-field">
            채널
            <select value={formChannel} onChange={(e) => setFormChannel(e.target.value as ReleaseChannel)}>
              {RELEASE_CHANNEL_OPTIONS.map((channel) => (
                <option key={channel} value={channel}>
                  {RELEASE_CHANNEL_LABEL[channel]}
                </option>
              ))}
            </select>
          </label>
          <label className="release-form-field">
            상태
            <select value={formStatus} onChange={(e) => setFormStatus(e.target.value as ReleaseStatus)}>
              {RELEASE_STATUS_OPTIONS.map((status) => (
                <option key={status} value={status}>
                  {RELEASE_STATUS_LABEL[status]}
                </option>
              ))}
            </select>
          </label>
        </div>
        <label className="release-form-field">
          노트
          <textarea
            value={formNotes}
            onChange={(e) => setFormNotes(e.target.value)}
            placeholder="심사 코멘트, 체크리스트 등 자유롭게 적어두세요."
          />
        </label>

        {editing && editing.notes && (
          <div className="card-actions">
            <button type="button" className="btn-text-secondary" onClick={handleCopyNotes}>
              {copiedNotes ? "복사됨" : "노트 복사"}
            </button>
          </div>
        )}

        {formError && (
          <div className="banner-error">
            <CheckStatusIcon status="fail" />
            <span>{formError}</span>
          </div>
        )}

        <div className="card-actions">
          <button type="button" className="btn btn-outline" disabled={saving} onClick={closeForm}>
            취소
          </button>
          {editing && (
            <button
              type="button"
              className="btn-danger-text"
              disabled={saving}
              onClick={() => setConfirmDelete(editing)}
            >
              삭제
            </button>
          )}
          <button
            type="button"
            className="btn btn-primary"
            disabled={saving || !formVersion.trim()}
            onClick={() => void handleSave()}
          >
            {saving && <SpinnerIcon />}
            {saving ? "저장하는 중…" : "저장"}
          </button>
        </div>
      </Modal>

      <Modal open={confirmDelete !== null} onClose={() => setConfirmDelete(null)} title="릴리스 삭제">
        {confirmDelete && (
          <>
            <p className="confirm-remove-text">
              <strong>{confirmDelete.version}</strong> 릴리스 기록을 삭제할까요? 이 기록만 지워지고 실제
              스토어 상태나 프로젝트 파일은 그대로예요.
            </p>
            {deleteError && (
              <div className="banner-error">
                <CheckStatusIcon status="fail" />
                <span>{deleteError}</span>
              </div>
            )}
            <div className="card-actions">
              <button type="button" className="btn btn-outline" disabled={deleting} onClick={() => setConfirmDelete(null)}>
                취소
              </button>
              <button
                type="button"
                className="btn btn-danger"
                disabled={deleting}
                onClick={() => void handleDelete(confirmDelete)}
              >
                {deleting && <SpinnerIcon />}
                {deleting ? "삭제하는 중…" : "삭제"}
              </button>
            </div>
          </>
        )}
      </Modal>
    </div>
  );
}
