import { useCallback, useMemo, useState } from "react";

export type TerminalTab = {
  id: string;
  title: string;
};

type UseTerminalTabsOptions = {
  activeWorkspaceId: string | null;
  onCloseTerminal?: (workspaceId: string, terminalId: string) => void;
};

function createTerminalId() {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `terminal-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export function useTerminalTabs({
  activeWorkspaceId,
  onCloseTerminal,
}: UseTerminalTabsOptions) {
  const [terminalByWorkspace, setTerminalByWorkspace] = useState<
    Record<string, TerminalTab>
  >({});

  const createTerminal = useCallback((workspaceId: string) => {
    const id = createTerminalId();
    setTerminalByWorkspace((prev) => ({
      ...prev,
      [workspaceId]: {
        id,
        title: "Terminal",
      },
    }));
    return id;
  }, []);

  const ensureTerminalWithTitle = useCallback(
    (workspaceId: string, terminalId: string, title: string) => {
      setTerminalByWorkspace((prev) => ({
        ...prev,
        [workspaceId]: {
          id: terminalId,
          title,
        },
      }));
      return terminalId;
    },
    [],
  );

  const closeTerminal = useCallback(
    (workspaceId: string, terminalId: string) => {
      setTerminalByWorkspace((prev) => {
        const existing = prev[workspaceId];
        if (!existing || existing.id !== terminalId) {
          return prev;
        }
        const { [workspaceId]: _, ...rest } = prev;
        return rest;
      });
      onCloseTerminal?.(workspaceId, terminalId);
    },
    [onCloseTerminal],
  );

  const setActiveTerminal = useCallback((workspaceId: string, terminalId: string) => {
    setTerminalByWorkspace((prev) => {
      const existing = prev[workspaceId];
      if (!existing || existing.id === terminalId) {
        return prev;
      }
      return {
        ...prev,
        [workspaceId]: {
          ...existing,
          id: terminalId,
        },
      };
    });
  }, []);

  const ensureTerminal = useCallback(
    (workspaceId: string) => {
      const existing = terminalByWorkspace[workspaceId];
      if (existing) {
        return existing.id;
      }
      return createTerminal(workspaceId);
    },
    [createTerminal, terminalByWorkspace],
  );

  const terminals = useMemo(() => {
    if (!activeWorkspaceId) {
      return [];
    }
    const terminal = terminalByWorkspace[activeWorkspaceId];
    return terminal ? [terminal] : [];
  }, [activeWorkspaceId, terminalByWorkspace]);

  const activeTerminalId = useMemo(() => {
    if (!activeWorkspaceId) {
      return null;
    }
    return terminalByWorkspace[activeWorkspaceId]?.id ?? null;
  }, [activeWorkspaceId, terminalByWorkspace]);

  return {
    terminals,
    activeTerminalId,
    createTerminal,
    ensureTerminalWithTitle,
    closeTerminal,
    setActiveTerminal,
    ensureTerminal,
  };
}
