import { memo, useEffect } from "react";

import type { ConversationItem, WorkspaceInfo } from "@/types";
import { useRuntimeAutoTaskRunner } from "@app/hooks/useRuntimeAutoTaskRunner";
import type { useRuntimeRegistry } from "@app/hooks/useRuntimeRegistry";
import type { RuntimeAutomationController } from "@app/runtime/runtimeHost";
import type { RuntimeId, RuntimeState } from "@app/runtime/runtimeState";

type RuntimeRegistry = ReturnType<typeof useRuntimeRegistry>;

type RuntimeAutomationBridgeProps = {
  runtime: RuntimeState;
  registry: RuntimeRegistry;
  workspacesById: Map<string, WorkspaceInfo>;
  items: ConversationItem[];
  isProcessing: boolean;
  isBlocked: boolean;
  timeoutMs: number;
  pauseAfterCompletionMs?: number;
  onDispatchTask: (runtimeId: RuntimeId, text: string) => void | Promise<void>;
  onControllerChange: (
    runtimeId: RuntimeId,
    controller: RuntimeAutomationController | null,
  ) => void;
};

function deriveLastTurnStatus({
  isProcessing,
  items,
}: {
  isProcessing: boolean;
  items: ConversationItem[];
}) {
  if (isProcessing) {
    return "running" as const;
  }
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (item.kind !== "message") {
      continue;
    }
    if (item.role === "assistant") {
      return "done" as const;
    }
    if (item.role === "user") {
      return "awaiting" as const;
    }
  }
  return "idle" as const;
}

export const RuntimeAutomationBridge = memo(function RuntimeAutomationBridge({
  runtime,
  registry,
  workspacesById,
  items,
  isProcessing,
  isBlocked,
  timeoutMs,
  pauseAfterCompletionMs,
  onDispatchTask,
  onControllerChange,
}: RuntimeAutomationBridgeProps) {
  const workspaceExists = workspacesById.has(runtime.workspaceId);
  const automationController = useRuntimeAutoTaskRunner({
    runtime,
    updateRuntime: registry.updateRuntime,
    isBlocked: isBlocked || !workspaceExists,
    timeoutMs,
    pauseAfterCompletionMs,
    onDispatchTask: (text) => onDispatchTask(runtime.id, text),
  });

  useEffect(() => {
    registry.updateRuntime(runtime.id, {
      threadId: runtime.threadId,
      execution: {
        isProcessing,
        lastTurnStatus: deriveLastTurnStatus({ isProcessing, items }),
      },
    });
  }, [isProcessing, items, registry, runtime.id, runtime.threadId]);

  useEffect(() => {
    onControllerChange(runtime.id, {
      enabled: runtime.automation.enabled,
      tasks: runtime.automation.tasks,
      summary: automationController.summary,
      setEnabled: automationController.setAutomationEnabled,
      importTasks: automationController.importTasks,
      importTasksFromText: automationController.importTasksFromText,
      clearAutomationTasks: automationController.clearAutomationTasks,
    });
    return () => {
      onControllerChange(runtime.id, null);
    };
  }, [
    automationController.clearAutomationTasks,
    automationController.importTasks,
    automationController.importTasksFromText,
    automationController.setAutomationEnabled,
    automationController.summary,
    onControllerChange,
    runtime.automation.enabled,
    runtime.automation.tasks,
    runtime.id,
  ]);

  return null;
});
