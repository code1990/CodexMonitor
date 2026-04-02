import { useEffect, useMemo, useRef } from "react";
import type { ConversationItem } from "@/types";
import { createRuntimeState, type RuntimeExecutionState } from "@app/runtime/runtimeState";
import type { RuntimeHostState } from "@app/runtime/runtimeHost";
import { useRuntimeAutoTaskRunner } from "@app/hooks/useRuntimeAutoTaskRunner";
import type { useRuntimeRegistry } from "@app/hooks/useRuntimeRegistry";

type RuntimeRegistry = ReturnType<typeof useRuntimeRegistry>;

type UseRuntimeHostArgs = {
  registry: RuntimeRegistry;
  workspaceId: string | null;
  threadId: string | null;
  draftText: string;
  isProcessing: boolean;
  items: ConversationItem[];
  automation: {
    isBlocked: boolean;
    timeoutMs: number;
    pauseAfterCompletionMs?: number;
    onDispatchTask: (text: string) => void | Promise<void>;
  };
};

function deriveLastTurnStatus({
  isProcessing,
  items,
}: {
  isProcessing: boolean;
  items: ConversationItem[];
}): RuntimeExecutionState["lastTurnStatus"] {
  if (isProcessing) {
    return "running";
  }
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (item.kind !== "message") {
      continue;
    }
    if (item.role === "assistant") {
      return "done";
    }
    if (item.role === "user") {
      return "awaiting";
    }
  }
  return "idle";
}

export function useRuntimeHost({
  registry,
  workspaceId,
  threadId,
  draftText,
  isProcessing,
  items,
  automation,
}: UseRuntimeHostArgs): RuntimeHostState {
  const fallbackRuntimeRef = useRef(
    createRuntimeState({
      id: "runtime:detached:__draft__",
      workspaceId: "detached",
      threadId: null,
    }),
  );
  const runtimeId = useMemo(() => {
    if (!workspaceId) {
      return null;
    }
    const suffix = threadId?.trim() ? threadId.trim() : "__draft__";
    return `runtime:${workspaceId}:${suffix}`;
  }, [threadId, workspaceId]);

  useEffect(() => {
    if (!workspaceId) {
      return;
    }
    registry.openRuntime({
      workspaceId,
      threadId,
    });
  }, [registry.openRuntime, threadId, workspaceId]);

  useEffect(() => {
    if (!runtimeId) {
      return;
    }
    registry.activateRuntime(runtimeId);
  }, [registry.activateRuntime, runtimeId]);

  const runtime = runtimeId
    ? registry.runtimesById[runtimeId] ?? fallbackRuntimeRef.current
    : null;
  const lastTurnStatus = useMemo(
    () => deriveLastTurnStatus({ isProcessing, items }),
    [isProcessing, items],
  );

  useEffect(() => {
    if (!runtimeId) {
      return;
    }
    registry.updateRuntime(runtimeId, {
      threadId,
      view: {
        draftText,
      },
      execution: {
        isProcessing,
        lastTurnStatus,
      },
    });
  }, [
    draftText,
    isProcessing,
    lastTurnStatus,
    registry.updateRuntime,
    runtimeId,
    threadId,
  ]);

  const automationController = useRuntimeAutoTaskRunner({
    runtime: runtime ?? fallbackRuntimeRef.current,
    updateRuntime: registry.updateRuntime,
    isBlocked: automation.isBlocked || !runtimeId,
    timeoutMs: automation.timeoutMs,
    pauseAfterCompletionMs: automation.pauseAfterCompletionMs,
    onDispatchTask: automation.onDispatchTask,
  });

  return {
    runtimeId,
    runtime,
    automation: runtimeId
      ? {
          enabled: runtime?.automation.enabled ?? false,
          tasks: automationController.tasks,
          summary: automationController.summary,
          setEnabled: automationController.setAutomationEnabled,
          importTasks: automationController.importTasks,
          importTasksFromText: automationController.importTasksFromText,
          clearAutomationTasks: automationController.clearAutomationTasks,
        }
      : null,
  };
}
