import { useCallback, useEffect } from "react";
import {
  useAutoTaskRunner,
  type AutoTaskImportInput,
} from "@/features/composer/hooks/useAutoTaskRunner";
import type { RuntimeId, RuntimeState } from "@app/runtime/runtimeState";

type UseRuntimeAutoTaskRunnerArgs = {
  runtime: RuntimeState;
  updateRuntime: (
    runtimeId: RuntimeId,
    patch: Partial<Omit<RuntimeState, "id" | "createdAt">>,
  ) => void;
  isBlocked: boolean;
  timeoutMs: number;
  pauseAfterCompletionMs?: number;
  onDispatchTask: (text: string) => void | Promise<void>;
};

export function useRuntimeAutoTaskRunner({
  runtime,
  updateRuntime,
  isBlocked,
  timeoutMs,
  pauseAfterCompletionMs,
  onDispatchTask,
}: UseRuntimeAutoTaskRunnerArgs) {
  const runner = useAutoTaskRunner({
    enabled: runtime.automation.enabled,
    isProcessing: runtime.execution.isProcessing,
    isBlocked,
    timeoutMs,
    pauseAfterCompletionMs,
    scopeKey: runtime.automation.scopeKey,
    onDispatchTask,
  });

  useEffect(() => {
    updateRuntime(runtime.id, {
      automation: {
        scopeKey: runtime.automation.scopeKey,
        enabled: runtime.automation.enabled,
        sourceName: runner.summary.sourceName,
        tasks: runner.tasks,
        queueLength: runner.summary.total,
        pendingCount: runner.summary.pending,
        runningCount: runner.summary.running,
        completedCount: runner.summary.completed,
        failedCount: runner.summary.failed,
        timedOutCount: runner.summary.timedOut,
        hasTerminalIssue: runner.summary.hasTerminalIssue,
        activeTaskId: runner.summary.activeTaskId,
      },
    });
  }, [
    runner.summary.activeTaskId,
    runner.summary.completed,
    runner.summary.failed,
    runner.summary.hasTerminalIssue,
    runner.summary.pending,
    runner.summary.running,
    runner.summary.sourceName,
    runner.summary.timedOut,
    runner.summary.total,
    runtime.automation.enabled,
    runtime.automation.scopeKey,
    runtime.id,
    updateRuntime,
  ]);

  const setAutomationEnabled = useCallback(
    (enabled: boolean) => {
      updateRuntime(runtime.id, {
        automation: {
          ...runtime.automation,
          enabled,
        },
      });
    },
    [runtime.automation, runtime.id, updateRuntime],
  );

  const importTasks = useCallback(
    (label: string, taskInputs: AutoTaskImportInput[]) => {
      runner.importTasks(label, taskInputs);
    },
    [runner],
  );

  const importTasksFromText = useCallback(
    (fileName: string, content: string) => {
      runner.importTasksFromText(fileName, content);
    },
    [runner],
  );

  const clearAutomationTasks = useCallback(() => {
    runner.clearAutomationTasks();
    updateRuntime(runtime.id, {
      automation: {
        ...runtime.automation,
        tasks: [],
        sourceName: null,
        queueLength: 0,
        pendingCount: 0,
        runningCount: 0,
        completedCount: 0,
        failedCount: 0,
        timedOutCount: 0,
        hasTerminalIssue: false,
        activeTaskId: null,
      },
    });
  }, [runner, runtime.automation, runtime.id, updateRuntime]);

  return {
    ...runner,
    setAutomationEnabled,
    importTasks,
    importTasksFromText,
    clearAutomationTasks,
  };
}
