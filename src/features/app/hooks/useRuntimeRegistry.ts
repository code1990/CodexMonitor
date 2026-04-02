import { useCallback, useMemo, useState } from "react";
import {
  createRuntimeState,
  type RuntimeId,
  type RuntimeState,
} from "@app/runtime/runtimeState";

type RuntimeRegistryState = {
  activeRuntimeId: RuntimeId | null;
  order: RuntimeId[];
  runtimesById: Record<RuntimeId, RuntimeState>;
};

type OpenRuntimeInput = {
  workspaceId: string;
  threadId?: string | null;
};

type UpdateRuntimeInput = Partial<Omit<RuntimeState, "id" | "createdAt">>;

function createRuntimeId({ workspaceId, threadId }: OpenRuntimeInput) {
  const suffix = threadId?.trim() ? threadId.trim() : "__draft__";
  return `runtime:${workspaceId}:${suffix}`;
}

export function useRuntimeRegistry() {
  const [state, setState] = useState<RuntimeRegistryState>({
    activeRuntimeId: null,
    order: [],
    runtimesById: {},
  });

  const openRuntime = useCallback((input: OpenRuntimeInput) => {
    const runtimeId = createRuntimeId(input);
    setState((current) => {
      const existing = current.runtimesById[runtimeId];
      if (existing) {
        return {
          ...current,
          activeRuntimeId: runtimeId,
        };
      }

      const runtime = createRuntimeState({
        id: runtimeId,
        workspaceId: input.workspaceId,
        threadId: input.threadId ?? null,
      });

      return {
        activeRuntimeId: runtimeId,
        order: [...current.order, runtimeId],
        runtimesById: {
          ...current.runtimesById,
          [runtimeId]: runtime,
        },
      };
    });
    return runtimeId;
  }, []);

  const activateRuntime = useCallback((runtimeId: RuntimeId) => {
    setState((current) => {
      if (!current.runtimesById[runtimeId] || current.activeRuntimeId === runtimeId) {
        return current;
      }
      return {
        ...current,
        activeRuntimeId: runtimeId,
      };
    });
  }, []);

  const updateRuntime = useCallback((runtimeId: RuntimeId, patch: UpdateRuntimeInput) => {
    setState((current) => {
      const runtime = current.runtimesById[runtimeId];
      if (!runtime) {
        return current;
      }
      return {
        ...current,
        runtimesById: {
          ...current.runtimesById,
          [runtimeId]: {
            ...runtime,
            ...patch,
            updatedAt: Date.now(),
            view: patch.view ? { ...runtime.view, ...patch.view } : runtime.view,
            execution: patch.execution
              ? { ...runtime.execution, ...patch.execution }
              : runtime.execution,
            automation: patch.automation
              ? { ...runtime.automation, ...patch.automation }
              : runtime.automation,
          },
        },
      };
    });
  }, []);

  const closeRuntime = useCallback((runtimeId: RuntimeId) => {
    setState((current) => {
      if (!current.runtimesById[runtimeId]) {
        return current;
      }
      const nextRuntimesById = { ...current.runtimesById };
      delete nextRuntimesById[runtimeId];
      const nextOrder = current.order.filter((id) => id !== runtimeId);
      const nextActiveRuntimeId =
        current.activeRuntimeId === runtimeId
          ? nextOrder[nextOrder.length - 1] ?? null
          : current.activeRuntimeId;
      return {
        activeRuntimeId: nextActiveRuntimeId,
        order: nextOrder,
        runtimesById: nextRuntimesById,
      };
    });
  }, []);

  const runtimes = useMemo(
    () => state.order.map((runtimeId) => state.runtimesById[runtimeId]).filter(Boolean),
    [state.order, state.runtimesById],
  );

  const activeRuntime = state.activeRuntimeId
    ? state.runtimesById[state.activeRuntimeId] ?? null
    : null;

  return {
    activeRuntimeId: state.activeRuntimeId,
    activeRuntime,
    runtimes,
    runtimesById: state.runtimesById,
    openRuntime,
    activateRuntime,
    updateRuntime,
    closeRuntime,
  };
}

