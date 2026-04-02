import { useCallback } from "react";

import type { useRuntimeRegistry } from "@app/hooks/useRuntimeRegistry";

type RuntimeRegistry = ReturnType<typeof useRuntimeRegistry>;

type UseRuntimeNavigationArgs = {
  registry: RuntimeRegistry;
  clearDraftState: () => void;
  startNewAgentDraft: (workspaceId: string) => void;
  selectWorkspace: (workspaceId: string) => void;
  setActiveThreadId: (threadId: string | null, workspaceId?: string) => void;
  isCompact: boolean;
  setActiveTab: (tab: "home" | "projects" | "codex" | "git" | "log") => void;
};

export function useRuntimeNavigation({
  registry,
  clearDraftState,
  startNewAgentDraft,
  selectWorkspace,
  setActiveThreadId,
  isCompact,
  setActiveTab,
}: UseRuntimeNavigationArgs) {
  const openDraftRuntime = useCallback(
    (workspaceId: string) => {
      registry.openRuntime({ workspaceId, threadId: null });
      clearDraftState();
      startNewAgentDraft(workspaceId);
      selectWorkspace(workspaceId);
      setActiveThreadId(null, workspaceId);
      if (isCompact) {
        setActiveTab("codex");
      }
    },
    [
      clearDraftState,
      isCompact,
      registry,
      selectWorkspace,
      setActiveTab,
      setActiveThreadId,
      startNewAgentDraft,
    ],
  );

  const openThreadRuntime = useCallback(
    (workspaceId: string, threadId: string) => {
      registry.openRuntime({ workspaceId, threadId });
      clearDraftState();
      selectWorkspace(workspaceId);
      setActiveThreadId(threadId, workspaceId);
      if (isCompact) {
        setActiveTab("codex");
      }
    },
    [
      clearDraftState,
      isCompact,
      registry,
      selectWorkspace,
      setActiveTab,
      setActiveThreadId,
    ],
  );

  return {
    openDraftRuntime,
    openThreadRuntime,
  };
}
