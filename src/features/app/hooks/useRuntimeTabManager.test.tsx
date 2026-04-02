// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ThreadSummary, WorkspaceInfo } from "@/types";
import { useRuntimeRegistry } from "./useRuntimeRegistry";
import { useRuntimeTabManager } from "./useRuntimeTabManager";

const workspace: WorkspaceInfo = {
  id: "ws-1",
  name: "Workspace A",
  path: "D:/workspace-a",
  connected: true,
  accessMode: "full-access",
  repoInitialized: true,
  providerType: "local",
  providerLabel: null,
  providerMeta: null,
  settings: {},
};

describe("useRuntimeTabManager", () => {
  it("selects and closes runtimes via workspace/thread routing", () => {
    const selectWorkspace = vi.fn();
    const setActiveThreadId = vi.fn();
    const clearDraftState = vi.fn();
    const setActiveTab = vi.fn();
    const threadsByWorkspace: Record<string, ThreadSummary[]> = {
      "ws-1": [
        { id: "thread-1", name: "First thread", updatedAt: 1 },
        { id: "thread-2", name: "Second thread", updatedAt: 2 },
      ],
    };

    const { result } = renderHook(() => {
      const registry = useRuntimeRegistry();
      const tabManager = useRuntimeTabManager({
        registry,
        workspacesById: new Map([["ws-1", workspace]]),
        threadsByWorkspace,
        selectWorkspace,
        setActiveThreadId,
        clearDraftState,
        isCompact: false,
        setActiveTab,
      });
      return {
        registry,
        tabManager,
      };
    });

    act(() => {
      result.current.registry.openRuntime({ workspaceId: "ws-1", threadId: "thread-1" });
      result.current.registry.openRuntime({ workspaceId: "ws-1", threadId: "thread-2" });
    });

    expect(result.current.tabManager.tabs).toHaveLength(2);
    expect(result.current.tabManager.tabs[0]?.title).toBe("First thread");
    expect(result.current.tabManager.tabs[1]?.isActive).toBe(true);

    act(() => {
      result.current.tabManager.selectRuntime("runtime:ws-1:thread-1");
    });

    expect(selectWorkspace).toHaveBeenLastCalledWith("ws-1");
    expect(setActiveThreadId).toHaveBeenLastCalledWith("thread-1", "ws-1");

    act(() => {
      result.current.tabManager.closeRuntime("runtime:ws-1:thread-1");
    });

    expect(result.current.tabManager.tabs).toHaveLength(1);
    expect(result.current.tabManager.tabs[0]?.title).toBe("Second thread");
  });
});
