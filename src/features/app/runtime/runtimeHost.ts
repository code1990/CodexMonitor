import type {
  AutoTaskImportInput,
  AutoTaskItem,
  AutoTaskSummary,
} from "@/features/composer/hooks/useAutoTaskRunner";
import type { RuntimeId, RuntimeState } from "@app/runtime/runtimeState";

export type RuntimeAutomationController = {
  enabled: boolean;
  promptEnabled: boolean;
  promptText: string;
  promptSourceName: string | null;
  tasks: AutoTaskItem[];
  summary: AutoTaskSummary;
  setEnabled: (enabled: boolean) => void;
  setPromptEnabled: (enabled: boolean) => void;
  setPromptText: (text: string) => void;
  setPromptSourceName: (name: string | null) => void;
  importTasks: (label: string, taskInputs: AutoTaskImportInput[]) => void;
  appendTasks: (label: string, taskInputs: AutoTaskImportInput[]) => void;
  importTasksFromText: (fileName: string, content: string) => void;
  clearAutomationTasks: () => void;
};

export type RuntimeHostState = {
  runtimeId: RuntimeId | null;
  runtime: RuntimeState | null;
  automation: RuntimeAutomationController | null;
};
