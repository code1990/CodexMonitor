import { useCallback, useEffect, useMemo, useRef, useState } from "react";

export type AutoTaskStatus =
  | "pending"
  | "dispatching"
  | "running"
  | "completed"
  | "failed"
  | "timed_out";

export type AutoTaskItem = {
  id: string;
  lineNumber: number;
  text: string;
  sourceFileName?: string | null;
  sourcePath?: string | null;
  exportFileName?: string | null;
  status: AutoTaskStatus;
  createdAt: number;
  startedAt: number | null;
  completedAt: number | null;
  error: string | null;
};

type UseAutoTaskRunnerOptions = {
  enabled: boolean;
  isProcessing: boolean;
  isBlocked: boolean;
  timeoutMs: number;
  pauseAfterCompletionMs?: number;
  scopeKey?: string | null;
  onDispatchTask: (text: string) => void | Promise<void>;
};

export type AutoTaskSummary = {
  sourceName: string | null;
  total: number;
  pending: number;
  running: number;
  completed: number;
  failed: number;
  timedOut: number;
  hasTerminalIssue: boolean;
  activeTaskId: string | null;
};

const createTaskId = (lineNumber: number) =>
  `auto-task-${lineNumber}-${Math.random().toString(36).slice(2, 8)}`;

export type AutoTaskImportInput = {
  lineNumber: number;
  text: string;
  sourceFileName?: string | null;
  sourcePath?: string | null;
  exportFileName?: string | null;
};

const parseTasks = (content: string) =>
  content
    .split(/\r?\n/)
    .map((line, index) => ({
      text: line.trim(),
      lineNumber: index + 1,
    }))
    .filter((line) => line.text.length > 0);

export function useAutoTaskRunner({
  enabled,
  isProcessing,
  isBlocked,
  timeoutMs,
  pauseAfterCompletionMs = 1_000,
  scopeKey = null,
  onDispatchTask,
}: UseAutoTaskRunnerOptions) {
  const [tasks, setTasks] = useState<AutoTaskItem[]>([]);
  const [sourceName, setSourceName] = useState<string | null>(null);
  const activeTaskIdRef = useRef<string | null>(null);
  const previousProcessingRef = useRef(isProcessing);
  const timeoutRef = useRef<number | null>(null);
  const cooldownRef = useRef<number | null>(null);
  const [cooldownTick, setCooldownTick] = useState(0);

  const clearTimer = useCallback(() => {
    if (timeoutRef.current !== null) {
      window.clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
  }, []);

  const clearCooldown = useCallback(() => {
    if (cooldownRef.current !== null) {
      window.clearTimeout(cooldownRef.current);
      cooldownRef.current = null;
    }
  }, []);

  const clearAutomationTasks = useCallback(() => {
    clearTimer();
    clearCooldown();
    activeTaskIdRef.current = null;
    setTasks([]);
    setSourceName(null);
  }, [clearCooldown, clearTimer]);

  useEffect(() => {
    clearAutomationTasks();
  }, [clearAutomationTasks, scopeKey]);

  const importTasks = useCallback((label: string, taskInputs: AutoTaskImportInput[]) => {
    const now = Date.now();
    const nextTasks = taskInputs.map(
      ({ lineNumber, text, sourceFileName = null, sourcePath = null, exportFileName = null }) => ({
        id: createTaskId(lineNumber),
        lineNumber,
        text,
        sourceFileName,
        sourcePath,
        exportFileName,
        status: "pending" as const,
        createdAt: now,
        startedAt: null,
        completedAt: null,
        error: null,
      }),
    );
    clearTimer();
    clearCooldown();
    activeTaskIdRef.current = null;
    setSourceName(label);
    setTasks(nextTasks);
  }, [clearCooldown, clearTimer]);

  const importTasksFromText = useCallback((fileName: string, content: string) => {
    importTasks(fileName, parseTasks(content));
  }, [importTasks]);

  const summary = useMemo<AutoTaskSummary>(() => {
    let pending = 0;
    let running = 0;
    let completed = 0;
    let failed = 0;
    let timedOut = 0;
    for (const task of tasks) {
      if (task.status === "pending") {
        pending += 1;
      } else if (task.status === "dispatching" || task.status === "running") {
        running += 1;
      } else if (task.status === "completed") {
        completed += 1;
      } else if (task.status === "failed") {
        failed += 1;
      } else if (task.status === "timed_out") {
        timedOut += 1;
      }
    }
    return {
      sourceName,
      total: tasks.length,
      pending,
      running,
      completed,
      failed,
      timedOut,
      hasTerminalIssue: failed > 0 || timedOut > 0,
      activeTaskId: activeTaskIdRef.current,
    };
  }, [sourceName, tasks]);
  const hasTerminalIssue = summary.hasTerminalIssue;
  const isCoolingDown = cooldownRef.current !== null;

  useEffect(() => {
    if (
      !enabled ||
      isBlocked ||
      isProcessing ||
      isCoolingDown ||
      hasTerminalIssue ||
      activeTaskIdRef.current
    ) {
      return;
    }
    const nextTask = tasks.find((task) => task.status === "pending");
    if (!nextTask) {
      return;
    }

    const dispatchTask = async () => {
      const startedAt = Date.now();
      activeTaskIdRef.current = nextTask.id;
      setTasks((current) =>
        current.map((task) =>
          task.id === nextTask.id
            ? {
                ...task,
                status: "dispatching",
                startedAt,
                completedAt: null,
                error: null,
              }
            : task,
        ),
      );

      try {
        await onDispatchTask(nextTask.text);
      } catch (error) {
        clearTimer();
        activeTaskIdRef.current = null;
        const message =
          error instanceof Error ? error.message : "Failed to dispatch task.";
        setTasks((current) =>
          current.map((task) =>
            task.id === nextTask.id
              ? {
                  ...task,
                  status: "failed",
                  completedAt: Date.now(),
                  error: message,
                }
              : task,
          ),
        );
      }
    };

    void dispatchTask();
  }, [
    clearTimer,
    cooldownTick,
    enabled,
    hasTerminalIssue,
    isBlocked,
    isCoolingDown,
    isProcessing,
    onDispatchTask,
    tasks,
  ]);

  useEffect(() => {
    const wasProcessing = previousProcessingRef.current;
    previousProcessingRef.current = isProcessing;
    const activeTaskId = activeTaskIdRef.current;
    if (!activeTaskId) {
      return;
    }

    if (!wasProcessing && isProcessing) {
      setTasks((current) =>
        current.map((task) =>
          task.id === activeTaskId
            ? {
                ...task,
                status: "running",
                startedAt: task.startedAt ?? Date.now(),
                error: null,
              }
            : task,
        ),
      );
      return;
    }

    if (wasProcessing && !isProcessing) {
      clearTimer();
      activeTaskIdRef.current = null;
      clearCooldown();
      if (pauseAfterCompletionMs > 0) {
        cooldownRef.current = window.setTimeout(() => {
          cooldownRef.current = null;
          setCooldownTick((value) => value + 1);
        }, pauseAfterCompletionMs);
      }
      setTasks((current) =>
        current.map((task) =>
          task.id === activeTaskId &&
          (task.status === "dispatching" || task.status === "running")
            ? {
                ...task,
                status: "completed",
                completedAt: Date.now(),
                error: null,
              }
            : task,
        ),
      );
    }
  }, [clearCooldown, clearTimer, isProcessing, pauseAfterCompletionMs]);

  useEffect(() => {
    clearTimer();
    const activeTaskId = activeTaskIdRef.current;
    if (!activeTaskId || timeoutMs <= 0) {
      return;
    }
    const activeTask = tasks.find((task) => task.id === activeTaskId);
    if (!activeTask || (activeTask.status !== "dispatching" && activeTask.status !== "running")) {
      return;
    }

    timeoutRef.current = window.setTimeout(() => {
      activeTaskIdRef.current = null;
      setTasks((current) =>
        current.map((task) =>
          task.id === activeTaskId
            ? {
                ...task,
                status: "timed_out",
                completedAt: Date.now(),
                error: `Timed out after ${Math.max(1, Math.round(timeoutMs / 1000))}s.`,
              }
            : task,
        ),
      );
    }, timeoutMs);

    return clearTimer;
  }, [clearTimer, tasks, timeoutMs]);

  return {
    tasks,
    summary,
    importTasks,
    importTasksFromText,
    clearAutomationTasks,
  };
}
