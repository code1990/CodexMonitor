import type { MouseEvent as ReactMouseEvent, ReactNode } from "react";
import type { TerminalTab } from "../hooks/useTerminalTabs";

type TerminalDockProps = {
  isOpen: boolean;
  terminals: TerminalTab[];
  activeTerminalId: string | null;
  onSelectTerminal: (terminalId: string) => void;
  onNewTerminal: () => void;
  onCloseTerminal: (terminalId: string) => void;
  onResizeStart?: (event: ReactMouseEvent) => void;
  terminalNode: ReactNode;
};

export function TerminalDock({
  isOpen,
  terminals,
  activeTerminalId,
  onCloseTerminal,
  onResizeStart,
  terminalNode,
}: TerminalDockProps) {
  if (!isOpen) {
    return null;
  }

  const activeTitle = terminals[0]?.title ?? "Terminal";

  return (
    <section className="terminal-panel">
      {onResizeStart && (
        <div
          className="terminal-panel-resizer"
          role="separator"
          aria-orientation="horizontal"
          aria-label="Resize terminal panel"
          onMouseDown={onResizeStart}
        />
      )}
      <div className="terminal-header">
        <div className="terminal-tabs" aria-label="Terminal">
          <div className="terminal-tab active" aria-current="page">
            <span className="terminal-tab-label">{activeTitle}</span>
            {activeTerminalId && (
              <button
                className="terminal-tab-close"
                type="button"
                aria-label={`Close ${activeTitle}`}
                onClick={() => onCloseTerminal(activeTerminalId)}
              >
                x
              </button>
            )}
          </div>
        </div>
      </div>
      <div className="terminal-body">{terminalNode}</div>
    </section>
  );
}
