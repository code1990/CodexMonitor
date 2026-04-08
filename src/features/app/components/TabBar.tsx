import type { ReactNode } from "react";
import MessagesSquare from "lucide-react/dist/esm/icons/messages-square";

type TabKey = "home" | "projects" | "codex" | "git" | "log";

type TabBarProps = {
  activeTab: TabKey;
  onSelect: (tab: TabKey) => void;
};

const tabs: { id: TabKey; label: string; icon: ReactNode }[] = [
  { id: "codex", label: "Codex", icon: <MessagesSquare className="tabbar-icon" /> },
];

export function TabBar({ activeTab, onSelect }: TabBarProps) {
  return (
    <nav className="tabbar" aria-label="Primary">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          className={`tabbar-item ${activeTab === tab.id ? "active" : ""}`}
          onClick={() => onSelect(tab.id)}
          aria-current={activeTab === tab.id ? "page" : undefined}
        >
          {tab.icon}
          <span className="tabbar-label">{tab.label}</span>
        </button>
      ))}
    </nav>
  );
}
