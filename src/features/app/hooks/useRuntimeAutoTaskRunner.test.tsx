// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { useEffect, useRef } from "react";
import { describe, expect, it, vi } from "vitest";

import { createRuntimeState } from "@app/runtime/runtimeState";
import { useRuntimeRegistry } from "./useRuntimeRegistry";
import { useRuntimeAutoTaskRunner } from "./useRuntimeAutoTaskRunner";

describe("useRuntimeAutoTaskRunner", () => {
  it("binds automation queue state to the runtime scope", async () => {
    const onDispatchTask = vi.fn().mockResolvedValue(undefined);
    const { result, unmount } = renderHook(() => {
      const registry = useRuntimeRegistry();
      const fallbackRuntimeRef = useRef(
        createRuntimeState({
          id: "runtime:ws-1:thread-1",
          workspaceId: "ws-1",
          threadId: "thread-1",
        }),
      );

      useEffect(() => {
        registry.openRuntime({ workspaceId: "ws-1", threadId: "thread-1" });
      }, [registry.openRuntime]);

      const runtime = registry.activeRuntime ?? fallbackRuntimeRef.current;
      const automation = useRuntimeAutoTaskRunner({
        runtime,
        updateRuntime: registry.updateRuntime,
        isBlocked: false,
        timeoutMs: 10_000,
        pauseAfterCompletionMs: 0,
        onDispatchTask,
      });
      return {
        registry,
        runtimeId: runtime.id,
        automation,
      };
    });

    act(() => {
      result.current.automation.setAutomationEnabled(true);
    });

    act(() => {
      result.current.automation.importTasksFromText("tasks.txt", "first\nsecond");
    });

    await act(async () => {
      await Promise.resolve();
    });

    const runtime = result.current.registry.runtimesById[result.current.runtimeId];
    expect(runtime.automation.scopeKey).toBe(result.current.runtimeId);
    expect(runtime.automation.enabled).toBe(true);
    expect(runtime.automation.queueLength).toBe(2);
    expect(runtime.automation.pendingCount).toBe(1);
    expect(runtime.automation.runningCount).toBe(1);
    expect(runtime.automation.sourceName).toBe("tasks.txt");
    expect(onDispatchTask).toHaveBeenCalledWith("first");

    act(() => {
      result.current.automation.clearAutomationTasks();
    });
    unmount();
  });
});
