import type { ChangeEvent, RefObject } from "react";
import type { AutoTaskItem, AutoTaskStatus } from "../hooks/useAutoTaskRunner";

type ComposerAutomationPanelProps = {
  enabled: boolean;
  busy: boolean;
  blocked: boolean;
  timeoutSeconds: number;
  pauseSeconds: number;
  sourceName: string | null;
  directorySourceName: string | null;
  promptSourceName: string | null;
  promptEnabled: boolean;
  autoExportEnabled: boolean;
  downloadDirectory: string | null;
  tasks: AutoTaskItem[];
  summary: {
    total: number;
    pending: number;
    running: number;
    completed: number;
    failed: number;
    timedOut: number;
    hasTerminalIssue: boolean;
  };
  fileInputRef: RefObject<HTMLInputElement | null>;
  promptFileInputRef: RefObject<HTMLInputElement | null>;
  onToggleEnabled: (checked: boolean) => void;
  onTogglePromptEnabled: (checked: boolean) => void;
  onToggleAutoExportEnabled: (checked: boolean) => void;
  onOpenFilePicker: () => void;
  onOpenDirectoryPicker: () => void;
  onOpenPromptFilePicker: () => void;
  onPickDownloadDirectory: () => void | Promise<void>;
  onFileChange: (event: ChangeEvent<HTMLInputElement>) => void | Promise<void>;
  onPromptFileChange: (event: ChangeEvent<HTMLInputElement>) => void | Promise<void>;
  onClear: () => void;
  onTimeoutSecondsChange: (value: number) => void;
  onPauseSecondsChange: (value: number) => void;
};

const STATUS_LABELS: Record<AutoTaskStatus, string> = {
  pending: "Pending",
  dispatching: "Dispatching",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  timed_out: "Timed out",
};

function formatSummaryText({
  blocked,
  busy,
  enabled,
  sourceName,
  summary,
}: Pick<
  ComposerAutomationPanelProps,
  "blocked" | "busy" | "enabled" | "sourceName" | "summary"
>) {
  if (!sourceName || summary.total === 0) {
    return "Import a TXT file to enqueue one task per non-empty line.";
  }
  if (summary.hasTerminalIssue) {
    return `${sourceName}: automation paused after an error or timeout.`;
  }
  if (!enabled) {
    return `${sourceName}: automation ready, enable Auto to start dispatching.`;
  }
  if (blocked) {
    return `${sourceName}: automation is waiting for the composer to become available.`;
  }
  if (busy || summary.running > 0) {
    return `${sourceName}: task in progress, waiting for the current interaction to finish.`;
  }
  if (summary.pending > 0) {
    return `${sourceName}: ready to dispatch the next pending task.`;
  }
  return `${sourceName}: all tasks completed.`;
}

export function ComposerAutomationPanel({
  enabled,
  busy,
  blocked,
  timeoutSeconds,
  pauseSeconds,
  sourceName,
  directorySourceName,
  promptSourceName,
  promptEnabled,
  autoExportEnabled,
  downloadDirectory,
  tasks,
  summary,
  fileInputRef,
  promptFileInputRef,
  onToggleEnabled,
  onTogglePromptEnabled,
  onToggleAutoExportEnabled,
  onOpenFilePicker,
  onOpenDirectoryPicker,
  onOpenPromptFilePicker,
  onPickDownloadDirectory,
  onFileChange,
  onPromptFileChange,
  onClear,
  onTimeoutSecondsChange,
  onPauseSecondsChange,
}: ComposerAutomationPanelProps) {
  return (
    <section className="composer-automation" aria-label="Automation tasks">
      <div className="composer-automation-bar">
        <label className="composer-automation-toggle">
          <input
            type="checkbox"
            checked={enabled}
            disabled={summary.total === 0 || summary.hasTerminalIssue}
            onChange={(event) => onToggleEnabled(event.target.checked)}
          />
          <span>Auto</span>
        </label>
        <button type="button" className="ghost" onClick={onOpenFilePicker}>
          Import TXT
        </button>
        <button type="button" className="ghost" onClick={onOpenDirectoryPicker}>
          MD Dir
        </button>
        <button type="button" className="ghost" onClick={onOpenPromptFilePicker}>
          Prompt TXT
        </button>
        <button type="button" className="ghost" onClick={() => void onPickDownloadDirectory()}>
          Download Dir
        </button>
        <label className="composer-automation-timeout">
          <span>Timeout(s)</span>
          <input
            type="number"
            min={5}
            step={5}
            value={timeoutSeconds}
            onChange={(event) =>
              onTimeoutSecondsChange(Number.parseInt(event.target.value, 10) || 5)
            }
          />
        </label>
        <label className="composer-automation-timeout">
          <span>Pause(s)</span>
          <input
            type="number"
            min={0}
            step={1}
            value={pauseSeconds}
            onChange={(event) =>
              onPauseSecondsChange(Math.max(0, Number.parseInt(event.target.value, 10) || 0))
            }
          />
        </label>
        <button
          type="button"
          className="ghost"
          disabled={summary.total === 0}
          onClick={onClear}
        >
          Clear
        </button>
        <input
          ref={fileInputRef}
          className="composer-automation-file"
          type="file"
          accept=".txt,text/plain"
          onChange={onFileChange}
        />
        <input
          ref={promptFileInputRef}
          className="composer-automation-file"
          type="file"
          accept=".txt,text/plain"
          onChange={onPromptFileChange}
        />
      </div>
      <div className="composer-automation-flags">
        <label className="composer-automation-toggle">
          <input
            type="checkbox"
            checked={promptEnabled}
            disabled={!promptSourceName}
            onChange={(event) => onTogglePromptEnabled(event.target.checked)}
          />
          <span>Use prompt TXT</span>
        </label>
        <label className="composer-automation-toggle">
          <input
            type="checkbox"
            checked={autoExportEnabled}
            disabled={!downloadDirectory}
            onChange={(event) => onToggleAutoExportEnabled(event.target.checked)}
          />
          <span>Auto export</span>
        </label>
      </div>
      <div className="composer-automation-paths">
        <span>Tasks: {sourceName ?? "-"}</span>
        <span>MD dir: {directorySourceName ?? "-"}</span>
        <span>Prompt: {promptSourceName ?? "-"}</span>
        <span>Download: {downloadDirectory ?? "-"}</span>
      </div>
      <div className="composer-automation-summary" role="status" aria-live="polite">
        {formatSummaryText({ blocked, busy, enabled, sourceName, summary })}
      </div>
      {tasks.length > 0 ? (
        <div className="composer-automation-table-wrap">
          <table className="composer-automation-table">
            <thead>
              <tr>
                <th>#</th>
                <th>Status</th>
                <th>Task</th>
                <th>Info</th>
              </tr>
            </thead>
            <tbody>
              {tasks.map((task) => (
                <tr key={task.id}>
                  <td>{task.lineNumber}</td>
                  <td>
                    <span
                      className={`composer-automation-badge is-${task.status}`}
                    >
                      {STATUS_LABELS[task.status]}
                    </span>
                  </td>
                  <td className="composer-automation-task">
                    {task.sourceFileName ? `${task.sourceFileName}` : task.text}
                  </td>
                  <td className="composer-automation-info">
                    {task.error ??
                      (task.status === "completed"
                        ? "Done"
                        : task.status === "running"
                          ? "Awaiting model response"
                          : task.status === "dispatching"
                            ? "Submitting"
                            : task.sourcePath ?? "-")}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
    </section>
  );
}
