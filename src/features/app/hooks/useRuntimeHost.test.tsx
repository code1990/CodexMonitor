// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ConversationItem } from "@/types";
import { useRuntimeHost } from "./useRuntimeHost";
import { useRuntimeRegistry } from "./useRuntimeRegistry";

const assistantMessage = (text: string): ConversationItem => ({
  kind: "message",
  role: "assistant",
  text,
});

describe("useRuntimeHost", () => {
  it("syncs draft and execution state into the active runtime", async () => {
    const onDispatchTask = vi.fn().mockResolvedValue(undefined);
    const { result, rerender } = renderHook(
      ({
        draftText,
        isProcessing,
        items,
      }: {
        draftText: string;
        isProcessing: boolean;
        items: ConversationItem[];
      }) => {
        const registry = useRuntimeRegistry();
        const runtimeHost = useRuntimeHost({
          registry,
          workspaceId: "ws-1",
          threadId: "thread-1",
          draftText,
          isProcessing,
          items,
          automation: {
            isBlocked: false,
            timeoutMs: 10_000,
            pauseAfterCompletionMs: 0,
            onDispatchTask,
          },
        });
        return {
          registry,
          runtimeHost,
        };
      },
      {
        initialProps: {
          draftText: "draft a",
          isProcessing: true,
          items: [],
        },
      },
    );

    await act(async () => {
      await Promise.resolve();
    });

    let runtime = result.current.registry.activeRuntime;
    expect(runtime?.view.draftText).toBe("draft a");
    expect(runtime?.execution.isProcessing).toBe(true);
    expect(runtime?.execution.lastTurnStatus).toBe("running");

    rerender({
      draftText: "draft b",
      isProcessing: false,
      items: [assistantMessage("done")],
    });

    await act(async () => {
      await Promise.resolve();
    });

    runtime = result.current.registry.activeRuntime;
    expect(result.current.runtimeHost.runtimeId).toBe("runtime:ws-1:thread-1");
    expect(runtime?.view.draftText).toBe("draft b");
    expect(runtime?.execution.isProcessing).toBe(false);
    expect(runtime?.execution.lastTurnStatus).toBe("done");
  });
});
