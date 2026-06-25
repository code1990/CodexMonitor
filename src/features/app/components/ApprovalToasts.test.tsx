// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ApprovalRequest, WorkspaceInfo } from "../../../types";
import { ApprovalToasts } from "./ApprovalToasts";

const workspaces: WorkspaceInfo[] = [
  {
    id: "workspace-1",
    name: "Workspace One",
    path: "/tmp/workspace-1",
    connected: true,
    settings: { sidebarCollapsed: false },
  },
];

const approvals: ApprovalRequest[] = [
  {
    workspace_id: "workspace-1",
    request_id: 1,
    method: "codex/requestApproval/shell",
    params: { command: "echo one" },
  },
  {
    workspace_id: "workspace-1",
    request_id: 2,
    method: "codex/requestApproval/shell",
    params: { command: "echo two" },
  },
];

describe("ApprovalToasts", () => {
  afterEach(() => {
    vi.useRealTimers();
    cleanup();
  });

  it("renders live-region semantics and handles Enter on primary request", () => {
    const onDecision = vi.fn();
    render(
      <ApprovalToasts approvals={approvals} workspaces={workspaces} onDecision={onDecision} />,
    );

    const region = screen.getByRole("region");
    expect(region.getAttribute("aria-live")).toBe("assertive");
    expect(screen.getAllByRole("alert")).toHaveLength(2);

    fireEvent.keyDown(window, { key: "Enter" });
    expect(onDecision).toHaveBeenCalledWith(approvals[1], "accept");
  });

  it("does not submit when an input is focused", () => {
    const onDecision = vi.fn();
    render(
      <ApprovalToasts approvals={approvals} workspaces={workspaces} onDecision={onDecision} />,
    );

    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    fireEvent.keyDown(window, { key: "Enter" });
    expect(onDecision).not.toHaveBeenCalled();
    document.body.removeChild(input);
  });

  it("auto-approves after 5 seconds when remember is unavailable", () => {
    vi.useFakeTimers();
    const onDecision = vi.fn();
    render(
      <ApprovalToasts approvals={approvals} workspaces={workspaces} onDecision={onDecision} />,
    );

    expect(screen.getByRole("button", { name: "Approve (5s)" })).toBeTruthy();

    act(() => {
      vi.advanceTimersByTime(5_000);
    });

    expect(onDecision).toHaveBeenCalledWith(approvals[1], "accept");
  });

  it("auto-remembers allowed commands after 5 seconds when available", () => {
    vi.useFakeTimers();
    const onDecision = vi.fn();
    const onRemember = vi.fn();
    render(
      <ApprovalToasts
        approvals={approvals}
        workspaces={workspaces}
        onDecision={onDecision}
        onRemember={onRemember}
      />,
    );

    act(() => {
      vi.advanceTimersByTime(5_000);
    });

    expect(onRemember).toHaveBeenCalledTimes(1);
    expect(onDecision).not.toHaveBeenCalled();
  });
});
