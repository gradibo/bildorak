// App.tsx — 빌도락 메인 화면. 사이드바 + 앱 카드 그리드(read-only 유닛: 등록 + preflight 점검만).
// 로컬 빌드 실행 버튼은 이번 유닛에 없다(향후 로드맵에서 추가 예정).

import { useEffect, useState } from "react";
import "./App.css";
import { Sidebar } from "./components/Sidebar";
import { ProjectCard } from "./components/ProjectCard";
import { SettingsView } from "./components/SettingsView";
import { UpdateModal } from "./components/UpdateModal";
import { CheckStatusIcon } from "./components/Icons";
import {
  listProjects,
  listSigningKeys,
  pickProjectFolder,
  registerProject,
} from "./lib/api";
import { useSettings } from "./lib/settings-context";
import type { AppView, ProjectRecord, SigningKeyRecord } from "./lib/types";

function App() {
  const { t } = useSettings();
  const [view, setView] = useState<AppView>("projects");
  const [projects, setProjects] = useState<ProjectRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [signingKeys, setSigningKeys] = useState<SigningKeyRecord[]>([]);

  useEffect(() => {
    listProjects()
      .then(setProjects)
      .catch((e) => setError(typeof e === "string" ? e : "등록된 프로젝트 목록을 불러오지 못했어요."))
      .finally(() => setLoading(false));
  }, []);

  // 서명키 관리도 무료다(commands.rs::signing_base_dir) — listProjects 와 마찬가지로 최초 진입 시
  // 한 번만 불러온다.
  useEffect(() => {
    listSigningKeys()
      .then(setSigningKeys)
      .catch(() => {});
  }, []);

  const handleAddProject = async () => {
    if (adding) return;
    setAdding(true);
    setError(null);
    try {
      const folderToken = await pickProjectFolder();
      if (!folderToken) return; // 사용자가 다이얼로그를 취소함
      const project = await registerProject(folderToken);
      setProjects((prev) => [...prev, project]);
    } catch (e) {
      setError(typeof e === "string" ? e : "프로젝트를 등록하지 못했어요. 잠시 후 다시 시도해 주세요.");
    } finally {
      setAdding(false);
    }
  };

  const handleRemoved = (id: string) => {
    setProjects((prev) => prev.filter((p) => p.id !== id));
  };

  // 서명키 목록 갱신 3종 — 실제 Rust 호출은 SigningKeysSection 이 하고, 여기는 그 결과로 전역
  // signingKeys 배열만 갱신한다(handleRemoved 가 projects 배열을 갱신만 하는 것과 같은 역할 분담).
  const handleSigningKeyRegistered = (key: SigningKeyRecord) => {
    setSigningKeys((prev) => [...prev, key]);
  };

  const handleSigningKeyRemoved = (keyId: string) => {
    setSigningKeys((prev) => prev.filter((k) => k.id !== keyId));
  };

  const handleSigningKeyUpdated = (key: SigningKeyRecord) => {
    setSigningKeys((prev) => prev.map((k) => (k.id === key.id ? key : k)));
  };

  // 서명키 카드의 "다른 앱에서도 사용 중" 표시용 — id → name 조회만 하면 되므로 프로젝트가 바뀔
  // 때마다 새로 만들어도 비용이 작다(개인 데스크톱 앱, 프로젝트 수가 적음).
  const projectNamesById = Object.fromEntries(projects.map((p): [string, string] => [p.id, p.name]));

  return (
    <div className="app-shell">
      <UpdateModal />
      <Sidebar active={view} onSelect={setView} />
      <main className="main">
        {view === "settings" ? (
          <SettingsView />
        ) : (
          <>
            <div className="topbar">
              <div>
                <div className="page-eyebrow">{t("app.eyebrow")}</div>
                <h1 className="page-title">{t("app.title")}</h1>
                <p className="page-hint">
                  Flutter 프로젝트 폴더를 등록하면 빌드 준비 상태(도구·플랫폼 폴더·디스크 여유)를 점검할 수
                  있어요. 이 화면은 점검만 합니다. 빌드나 배포는 아직 실행하지 않아요.
                </p>
              </div>
              <div className="topbar-actions">
                <button type="button" className="btn btn-primary" disabled={adding} onClick={() => void handleAddProject()}>
                  {adding ? "등록하는 중…" : "프로젝트 폴더 추가"}
                </button>
              </div>
            </div>

            {error && (
              <div className="banner-error">
                <CheckStatusIcon status="fail" />
                <span>{error}</span>
              </div>
            )}

            {loading ? (
              <p style={{ color: "var(--text-3)", fontSize: 13 }}>불러오는 중…</p>
            ) : projects.length === 0 ? (
              <div className="card card-empty">
                <p>아직 등록된 프로젝트가 없어요.</p>
                <p className="empty-action">→ "프로젝트 폴더 추가"를 눌러 Flutter 프로젝트 폴더를 선택하세요.</p>
              </div>
            ) : (
              <div className="project-grid">
                {projects.map((project) => (
                  <ProjectCard
                    key={project.id}
                    project={project}
                    onRemoved={handleRemoved}
                    signingKeys={signingKeys}
                    projectNamesById={projectNamesById}
                    onSigningKeyRegistered={handleSigningKeyRegistered}
                    onSigningKeyRemoved={handleSigningKeyRemoved}
                    onSigningKeyUpdated={handleSigningKeyUpdated}
                  />
                ))}
              </div>
            )}
          </>
        )}
      </main>
    </div>
  );
}

export default App;
