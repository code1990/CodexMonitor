import type { AutoTaskItem } from "@/features/composer/hooks/useAutoTaskRunner";

export type RuntimeId = string;

export type RuntimeAutomationState = {
  enabled: boolean;
  scopeKey: string;
  sourceName: string | null;
  tasks: AutoTaskItem[];
  queueLength: number;
  pendingCount: number;
  runningCount: number;
  completedCount: number;
  failedCount: number;
  timedOutCount: number;
  hasTerminalIssue: boolean;
  activeTaskId: string | null;
};

export type RuntimeViewState = {
  draftText: string;
};

export type RuntimeExecutionState = {
  isProcessing: boolean;
  lastTurnStatus: "idle" | "running" | "awaiting" | "done" | "error";
};

export type RuntimeState = {
  id: RuntimeId;
  workspaceId: string;
  threadId: string | null;
  createdAt: number;
  updatedAt: number;
  view: RuntimeViewState;
  execution: RuntimeExecutionState;
  automation: RuntimeAutomationState;
};

export type CreateRuntimeStateInput = {
  id: RuntimeId;
  workspaceId: string;
  threadId?: string | null;
  now?: number;
};

export function createRuntimeState({
  id,
  workspaceId,
  threadId = null,
  now = Date.now(),
}: CreateRuntimeStateInput): RuntimeState {
  return {
    id,
    workspaceId,
    threadId,
    createdAt: now,
    updatedAt: now,
    view: {
      draftText: "",
    },
    execution: {
      isProcessing: false,
      lastTurnStatus: "idle",
    },
    automation: {
      enabled: false,
      scopeKey: id,
      sourceName: null,
      tasks: [],
      queueLength: 0,
      pendingCount: 0,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      timedOutCount: 0,
      hasTerminalIssue: false,
      activeTaskId: null,
    },
  };
}
