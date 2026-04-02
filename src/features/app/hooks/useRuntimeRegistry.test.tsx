// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { useRuntimeRegistry } from "./useRuntimeRegistry";

describe("useRuntimeRegistry", () => {
  it("opens and activates a runtime per workspace/thread pair", () => {
    const { result } = renderHook(() => useRuntimeRegistry());

    let runtimeId = "";
    act(() => {
      runtimeId = result.current.openRuntime({
        workspaceId: "ws-1",
        threadId: "thread-1",
      });
    });

    expect(runtimeId).toBe("runtime:ws-1:thread-1");
    expect(result.current.activeRuntimeId).toBe(runtimeId);
    expect(result.current.runtimes).toHaveLength(1);
    expect(result.current.activeRuntime?.workspaceId).toBe("ws-1");
    expect(result.current.activeRuntime?.threadId).toBe("thread-1");
  });

  it("reuses an existing runtime for the same workspace/thread pair", () => {
    const { result } = renderHook(() => useRuntimeRegistry());

    let firstId = "";
    let secondId = "";
    act(() => {
      firstId = result.current.openRuntime({
        workspaceId: "ws-1",
        threadId: "thread-1",
      });
      secondId = result.current.openRuntime({
        workspaceId: "ws-1",
        threadId: "thread-1",
      });
    });

    expect(secondId).toBe(firstId);
    expect(result.current.runtimes).toHaveLength(1);
  });

  it("updates nested runtime state without replacing unrelated slices", () => {
    const { result } = renderHook(() => useRuntimeRegistry());

    let runtimeId = "";
    act(() => {
      runtimeId = result.current.openRuntime({
        workspaceId: "ws-1",
        threadId: "thread-1",
      });
    });

    act(() => {
      result.current.updateRuntime(runtimeId, {
        view: { draftText: "draft a" },
        automation: { enabled: true, queueLength: 3 },
      });
    });

    expect(result.current.activeRuntime?.view.draftText).toBe("draft a");
    expect(result.current.activeRuntime?.automation.enabled).toBe(true);
    expect(result.current.activeRuntime?.automation.queueLength).toBe(3);
    expect(result.current.activeRuntime?.execution.lastTurnStatus).toBe("idle");
  });

  it("falls back to the previous runtime when closing the active runtime", () => {
    const { result } = renderHook(() => useRuntimeRegistry());

    let firstId = "";
    let secondId = "";
    act(() => {
      firstId = result.current.openRuntime({
        workspaceId: "ws-1",
        threadId: "thread-1",
      });
      secondId = result.current.openRuntime({
        workspaceId: "ws-1",
        threadId: "thread-2",
      });
    });

    expect(result.current.activeRuntimeId).toBe(secondId);

    act(() => {
      result.current.closeRuntime(secondId);
    });

    expect(result.current.activeRuntimeId).toBe(firstId);
    expect(result.current.runtimes).toHaveLength(1);
  });
});
