import X from "lucide-react/dist/esm/icons/x";

export type RuntimeTabItem = {
  id: string;
  title: string;
  subtitle: string | null;
  isActive: boolean;
  isProcessing: boolean;
};

type RuntimeTabBarProps = {
  tabs: RuntimeTabItem[];
  onSelectTab: (runtimeId: string) => void;
  onCloseTab: (runtimeId: string) => void;
};

export function RuntimeTabBar({
  tabs,
  onSelectTab,
  onCloseTab,
}: RuntimeTabBarProps) {
  if (tabs.length === 0) {
    return null;
  }

  return (
    <div className="runtime-tabbar" role="tablist" aria-label="Open conversations">
      {tabs.map((tab) => (
        <div
          key={tab.id}
          className={`runtime-tab ${tab.isActive ? "is-active" : ""}`}
          role="tab"
          aria-selected={tab.isActive}
        >
          <button
            type="button"
            className="runtime-tab-trigger"
            onClick={() => onSelectTab(tab.id)}
          >
            <span className="runtime-tab-title-row">
              <span className="runtime-tab-title">{tab.title}</span>
              {tab.isProcessing ? <span className="runtime-tab-status" /> : null}
            </span>
            {tab.subtitle ? (
              <span className="runtime-tab-subtitle">{tab.subtitle}</span>
            ) : null}
          </button>
          <button
            type="button"
            className="runtime-tab-close"
            aria-label={`Close ${tab.title}`}
            onClick={() => onCloseTab(tab.id)}
          >
            <X className="runtime-tab-close-icon" />
          </button>
        </div>
      ))}
    </div>
  );
}
