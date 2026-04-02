// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { useRuntimeNavigation } from "./useRuntimeNavigation";
import { useRuntimeRegistry } from "./useRuntimeRegistry";

describe("useRuntimeNavigation", () => {
  it("opens a draft runtime and prepares draft state", () => {
    const clearDraftState = vi.fn();
    const startNewAgentDraft = vi.fn();
    const selectWorkspace = vi.fn();
    const setActiveThreadId = vi.fn();
    const setActiveTab = vi.fn();

    const { result } = renderHook(() => {
      const registry = useRuntimeRegistry();
      const navigation = useRuntimeNavigation({
        registry,
        clearDraftState,
        startNewAgentDraft,
        selectWorkspace,
        setActiveThreadId,
        isCompact: true,
        setActiveTab,
      });
      return { registry, navigation };
    });

    act(() => {
      result.current.navigation.openDraftRuntime("ws-1");
    });

    expect(result.current.registry.activeRuntimeId).toBe("runtime:ws-1:__draft__");
    expect(startNewAgentDraft).toHaveBeenCalledWith("ws-1");
    expect(selectWorkspace).toHaveBeenCalledWith("ws-1");
    expect(setActiveThreadId).toHaveBeenCalledWith(null, "ws-1");
    expect(setActiveTab).toHaveBeenCalledWith("codex");
  });

  it("opens a thread runtime and clears draft state", () => {
    const clearDraftState = vi.fn();
    const selectWorkspace = vi.fn();
    const setActiveThreadId = vi.fn();

    const { result } = renderHook(() => {
      const registry = useRuntimeRegistry();
      const navigation = useRuntimeNavigation({
        registry,
        clearDraftState,
        startNewAgentDraft: vi.fn(),
        selectWorkspace,
        setActiveThreadId,
        isCompact: false,
        setActiveTab: vi.fn(),
      });
      return { registry, navigation };
    });

    act(() => {
      result.current.navigation.openThreadRuntime("ws-1", "thread-1");
    });

    expect(result.current.registry.activeRuntimeId).toBe("runtime:ws-1:thread-1");
    expect(clearDraftState).toHaveBeenCalledTimes(1);
    expect(selectWorkspace).toHaveBeenCalledWith("ws-1");
    expect(setActiveThreadId).toHaveBeenCalledWith("thread-1", "ws-1");
  });
});
