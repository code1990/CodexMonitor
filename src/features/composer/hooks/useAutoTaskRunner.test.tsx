// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useAutoTaskRunner } from "./useAutoTaskRunner";

describe("useAutoTaskRunner", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("dispatches the next pending task when enabled and idle", async () => {
    const onDispatchTask = vi.fn().mockResolvedValue(undefined);
    const { result, rerender } = renderHook(
      (props: {
        enabled: boolean;
        isProcessing: boolean;
        isBlocked: boolean;
        timeoutMs: number;
      }) =>
        useAutoTaskRunner({
          ...props,
          scopeKey: "workspace-1",
          pauseAfterCompletionMs: 0,
          onDispatchTask,
        }),
      {
        initialProps: {
          enabled: false,
          isProcessing: false,
          isBlocked: false,
          timeoutMs: 10_000,
        },
      },
    );

    act(() => {
      result.current.importTasksFromText("tasks.txt", "first\nsecond");
    });

    await act(async () => {
      rerender({
        enabled: true,
        isProcessing: false,
        isBlocked: false,
        timeoutMs: 10_000,
      });
      await Promise.resolve();
    });

    expect(onDispatchTask).toHaveBeenCalledTimes(1);
    expect(onDispatchTask).toHaveBeenCalledWith("first");
    expect(result.current.tasks[0]?.status).toBe("dispatching");
    expect(result.current.summary.total).toBe(2);
  });

  it("marks tasks completed and advances after processing finishes", async () => {
    const onDispatchTask = vi.fn().mockResolvedValue(undefined);
    const { result, rerender } = renderHook(
      (props: {
        enabled: boolean;
        isProcessing: boolean;
      }) =>
        useAutoTaskRunner({
          enabled: props.enabled,
          isProcessing: props.isProcessing,
          isBlocked: false,
          timeoutMs: 10_000,
          pauseAfterCompletionMs: 0,
          scopeKey: "workspace-1",
          onDispatchTask,
        }),
      {
        initialProps: {
          enabled: true,
          isProcessing: false,
        },
      },
    );

    act(() => {
      result.current.importTasksFromText("tasks.txt", "first\nsecond");
    });

    await act(async () => {
      await Promise.resolve();
    });
    expect(onDispatchTask).toHaveBeenNthCalledWith(1, "first");

    act(() => {
      rerender({ enabled: true, isProcessing: true });
    });
    expect(result.current.tasks[0]?.status).toBe("running");

    await act(async () => {
      rerender({ enabled: true, isProcessing: false });
      await Promise.resolve();
    });

    expect(result.current.tasks[0]?.status).toBe("completed");
    expect(onDispatchTask).toHaveBeenNthCalledWith(2, "second");
  });

  it("marks the active task timed out when the timeout is reached", async () => {
    vi.useFakeTimers();
    const onDispatchTask = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() =>
      useAutoTaskRunner({
        enabled: true,
        isProcessing: false,
        isBlocked: false,
        timeoutMs: 1_000,
        pauseAfterCompletionMs: 0,
        scopeKey: "workspace-1",
        onDispatchTask,
      }),
    );

    act(() => {
      result.current.importTasksFromText("tasks.txt", "first");
    });

    await act(async () => {
      await Promise.resolve();
    });
    expect(result.current.tasks[0]?.status).toBe("dispatching");

    act(() => {
      vi.advanceTimersByTime(1_000);
    });

    expect(result.current.tasks[0]?.status).toBe("timed_out");
    expect(result.current.summary.hasTerminalIssue).toBe(true);
  });
});
