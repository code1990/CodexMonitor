import { useCallback, useMemo } from "react";
import type { ThreadSummary, WorkspaceInfo } from "@/types";
import type { RuntimeId } from "@app/runtime/runtimeState";
import type { RuntimeTabItem } from "@app/components/RuntimeTabBar";
import type { useRuntimeRegistry } from "@app/hooks/useRuntimeRegistry";

type RuntimeRegistry = ReturnType<typeof useRuntimeRegistry>;

type UseRuntimeTabManagerArgs = {
  registry: RuntimeRegistry;
  workspacesById: Map<string, WorkspaceInfo>;
  threadsByWorkspace: Record<string, ThreadSummary[]>;
  selectWorkspace: (workspaceId: string) => void;
  setActiveThreadId: (threadId: string | null, workspaceId?: string) => void;
  clearDraftState: () => void;
  isCompact: boolean;
  setActiveTab: (tab: "home" | "projects" | "codex" | "git" | "log") => void;
};

function getRuntimeTitle({
  workspaceName,
  threadName,
  threadId,
}: {
  workspaceName: string | null;
  threadName: string | null;
  threadId: string | null;
}) {
  if (threadName) {
    return threadName;
  }
  if (threadId) {
    return threadId;
  }
  return workspaceName ? `New chat` : "Conversation";
}

export function useRuntimeTabManager({
  registry,
  workspacesById,
  threadsByWorkspace,
  selectWorkspace,
  setActiveThreadId,
  clearDraftState,
  isCompact,
  setActiveTab,
}: UseRuntimeTabManagerArgs) {
  const tabs = useMemo<RuntimeTabItem[]>(() => {
    return registry.runtimes.map((runtime) => {
      const workspace = workspacesById.get(runtime.workspaceId);
      const threadSummary =
        runtime.threadId
          ? (threadsByWorkspace[runtime.workspaceId] ?? []).find(
              (thread) => thread.id === runtime.threadId,
            ) ?? null
          : null;

      return {
        id: runtime.id,
        title: getRuntimeTitle({
          workspaceName: workspace?.name ?? null,
          threadName: threadSummary?.name?.trim() || null,
          threadId: runtime.threadId,
        }),
        subtitle: workspace?.name ?? null,
        isActive: registry.activeRuntimeId === runtime.id,
        isProcessing: runtime.execution.isProcessing,
      };
    });
  }, [registry.activeRuntimeId, registry.runtimes, threadsByWorkspace, workspacesById]);

  const selectRuntime = useCallback(
    (runtimeId: RuntimeId) => {
      const runtime = registry.runtimesById[runtimeId];
      if (!runtime) {
        return;
      }
      registry.activateRuntime(runtimeId);
      clearDraftState();
      selectWorkspace(runtime.workspaceId);
      setActiveThreadId(runtime.threadId, runtime.workspaceId);
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

  const closeRuntime = useCallback(
    (runtimeId: RuntimeId) => {
      const runtimeIndex = registry.runtimes.findIndex((runtime) => runtime.id === runtimeId);
      if (runtimeIndex < 0) {
        return;
      }
      const closingRuntime = registry.runtimes[runtimeIndex];
      const isClosingActive = registry.activeRuntimeId === runtimeId;
      const fallbackRuntime =
        isClosingActive
          ? registry.runtimes[runtimeIndex - 1] ??
            registry.runtimes[runtimeIndex + 1] ??
            null
          : null;

      registry.closeRuntime(runtimeId);

      if (!isClosingActive) {
        return;
      }

      if (fallbackRuntime) {
        clearDraftState();
        selectWorkspace(fallbackRuntime.workspaceId);
        setActiveThreadId(fallbackRuntime.threadId, fallbackRuntime.workspaceId);
        if (isCompact) {
          setActiveTab("codex");
        }
        return;
      }

      clearDraftState();
      selectWorkspace(closingRuntime.workspaceId);
      setActiveThreadId(null, closingRuntime.workspaceId);
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
    tabs,
    selectRuntime,
    closeRuntime,
  };
}
