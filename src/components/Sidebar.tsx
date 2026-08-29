// Sidebar.tsx — 사이드바(프로젝트/설정 화면 전환). 이전엔 "설정"이 "곧" placeholder였다 — 이제
// SettingsView 로 전환하는 실제 버튼이다(App.tsx::view 상태를 부모가 들고 있다, ProjectCard 등 다른
// 화면들이 자기 상태를 부모에 올리는 것과 같은 패턴).

import { useSettings } from "../lib/settings-context";
import type { AppView } from "../lib/types";

export function Sidebar({ active, onSelect }: { active: AppView; onSelect: (view: AppView) => void }) {
  const { t } = useSettings();
  return (
    <aside className="sidebar">
      <div className="sidebar-brand">빌도락</div>
      <button
        type="button"
        className={`sidebar-item sidebar-item-button${active === "projects" ? " active" : ""}`}
        onClick={() => onSelect("projects")}
      >
        {t("nav.projects")}
      </button>
      <button
        type="button"
        className={`sidebar-item sidebar-item-button${active === "settings" ? " active" : ""}`}
        onClick={() => onSelect("settings")}
      >
        {t("nav.settings")}
      </button>
    </aside>
  );
}
