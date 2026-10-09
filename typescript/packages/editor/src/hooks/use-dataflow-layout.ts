/** Copyright (c) Scott A Dixon */
import React from "react";
import { DAGRE_ENGINE_DESCRIPTOR } from "../graph/layout/registry.js";
import type {
  LayoutGraph,
  LayoutPositions,
  LayoutRunOptions,
  LayoutEngineDescriptor,
} from "../graph/layout/types.js";
import type { AuthoringGraphAutoLayoutController } from "../graph.js";
import {
  buildDataflowLayoutGraph,
  applyDataflowLayout,
} from "../core/flow-layout.js";
import type { FlowStudioWorkspaceState } from "../core/flow-persistence.js";
import type { DataflowStructuralUndoState } from "./use-dataflow-structural-undo.js";
export interface DataflowLayoutOptions {
  workspace: FlowStudioWorkspaceState;
  commit: DataflowStructuralUndoState["commitLayoutChange"];
  run: (
    engineId: string,
    graph: LayoutGraph,
    options?: LayoutRunOptions,
  ) => Promise<LayoutPositions>;
  onError: (error: unknown) => void;
  engines?: readonly LayoutEngineDescriptor[];
}
export function useDataflowLayout({
  workspace,
  commit,
  run,
  onError,
  engines = [DAGRE_ENGINE_DESCRIPTOR],
}: DataflowLayoutOptions): AuthoringGraphAutoLayoutController {
  const [engineId, setEngineId] = React.useState(engines[0]?.id ?? "");
  const [running, setRunning] = React.useState(false);
  const current = React.useRef(workspace);
  current.current = workspace;
  const pending = React.useRef<AbortController | null>(null);
  React.useEffect(() => () => pending.current?.abort(), []);
  const apply = (id: string) => {
    pending.current?.abort();
    const controller = new AbortController();
    pending.current = controller;
    const snapshot = current.current;
    setRunning(true);
    void run(id, buildDataflowLayoutGraph(snapshot), {
      signal: controller.signal,
    })
      .then((positions) => {
        if (!controller.signal.aborted && current.current === snapshot)
          commit("Auto layout", (value) =>
            applyDataflowLayout(value, positions),
          );
      })
      .catch((error) => {
        if (!controller.signal.aborted) onError(error);
      })
      .finally(() => {
        if (pending.current === controller) {
          pending.current = null;
          setRunning(false);
        }
      });
  };
  return {
    activeEngineId: engineId,
    engines,
    disabled: workspace.nodes.length === 0,
    label: "Auto layout",
    onSelectEngine: setEngineId,
    onApply: apply,
    status: running ? "running" : "idle",
    visible: true,
  };
}
