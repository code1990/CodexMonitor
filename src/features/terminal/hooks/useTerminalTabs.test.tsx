// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useTerminalTabs } from "./useTerminalTabs";

describe("useTerminalTabs single-tab mode", () => {
  it("creates and activates a named terminal", () => {
    const { result } = renderHook(() =>
      useTerminalTabs({ activeWorkspaceId: "workspace-1" }),
    );

    act(() => {
      result.current.ensureTerminalWithTitle("workspace-1", "launch", "Launch");
    });

    expect(result.current.terminals).toEqual([{ id: "launch", title: "Launch" }]);
    expect(result.current.activeTerminalId).toBe("launch");
  });

  it("replaces the existing terminal instead of creating multiple tabs", () => {
    const { result } = renderHook(() =>
      useTerminalTabs({ activeWorkspaceId: "workspace-1" }),
    );

    let firstId = "";
    let secondId = "";
    act(() => {
      firstId = result.current.createTerminal("workspace-1");
      secondId = result.current.createTerminal("workspace-1");
    });

    expect(firstId).not.toBe(secondId);
    expect(result.current.terminals).toEqual([{ id: secondId, title: "Terminal" }]);
    expect(result.current.activeTerminalId).toBe(secondId);
  });

  it("removes the terminal when the only tab is closed", () => {
    const { result } = renderHook(() =>
      useTerminalTabs({ activeWorkspaceId: "workspace-1" }),
    );

    let terminalId = "";
    act(() => {
      terminalId = result.current.createTerminal("workspace-1");
    });

    act(() => {
      result.current.closeTerminal("workspace-1", terminalId);
    });

    expect(result.current.terminals).toEqual([]);
    expect(result.current.activeTerminalId).toBeNull();
  });
});
