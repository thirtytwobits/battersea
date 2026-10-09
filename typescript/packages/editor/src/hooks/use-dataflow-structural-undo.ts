/**
 * Copyright (c) Scott A Dixon
 *
 * Owns structural dataflow undo history and restore behaviour. Backed
 * directly by `react-amnesia` — keyboard routing comes from the editor
 * shell's multi-scope provider.
 */
import React from "react";
import { type Amnesia, useAmnesiaScope } from "react-amnesia";

import {
  type DataflowUndoHistoryState,
  areDataflowStructuralSnapshotsEqual,
  areDataflowStructuralSnapshotsExactlyEqual,
  buildDataflowStructuralSnapshot,
  buildDataflowUndoHistoryState,
  commitDataflowSnapshotChange,
  pushDataflowPresentSnapshotChange,
  restoreDataflowStructuralWorkspace,
  type DataflowStructuralSnapshot,
} from "../core/dataflow-undo.js";
import type { FlowStudioWorkspaceState } from "../core/flow-persistence.js";

export const DATAFLOW_STRUCTURAL_UNDO_SCOPE_ID = "dataflow:structural";

export interface DataflowStructuralChangeOptions {
  mergeKey?: string | null;
}

export interface DataflowStructuralUndoState {
  /**
   * The underlying amnesia store. Exposed so the tab's view can wrap its
   * subtree with `<AmnesiaProvider store={amnesia}>` + `<AmnesiaShortcuts>`.
   */
  amnesia: Amnesia;
  captureSnapshot: () => DataflowStructuralSnapshot;
  commitLayoutChange: (
    label: string,
    transform: (
      workspace: FlowStudioWorkspaceState,
    ) => FlowStudioWorkspaceState,
    options?: DataflowStructuralChangeOptions,
  ) => void;
  commitStructuralChange: (
    label: string,
    transform: (
      workspace: FlowStudioWorkspaceState,
    ) => FlowStudioWorkspaceState,
    options?: DataflowStructuralChangeOptions,
  ) => void;
  pushPresentLayoutChange: (
    label: string,
    previousSnapshot: DataflowStructuralSnapshot,
    options?: DataflowStructuralChangeOptions,
  ) => void;
  setTrackedWorkspace: React.Dispatch<
    React.SetStateAction<FlowStudioWorkspaceState>
  >;
  replaceWorkspaceAndResetUndo: (
    nextWorkspace: FlowStudioWorkspaceState,
  ) => void;
  undoSelectionResetToken: number;
}

export function useDataflowStructuralUndo(options: {
  setWorkspace: React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>;
  workspace: FlowStudioWorkspaceState;
}): DataflowStructuralUndoState {
  const amnesia = useAmnesiaScope(DATAFLOW_STRUCTURAL_UNDO_SCOPE_ID);

  const workspaceRef = React.useRef(options.workspace);
  workspaceRef.current = options.workspace;

  // Tracks the (snapshot, restoreLayout) pair amnesia last captured. Used
  // as the diff baseline for `commitStructuralChange` and updated by every
  // push / amend / clear / undo / redo. Lazy-init to avoid recomputing the
  // initial snapshot on each render.
  const lastCapturedStateRef = React.useRef<DataflowUndoHistoryState | null>(
    null,
  );
  if (lastCapturedStateRef.current === null) {
    lastCapturedStateRef.current = buildDataflowUndoHistoryState(
      buildDataflowStructuralSnapshot(options.workspace),
      false,
    );
  }

  const [undoSelectionResetToken, setUndoSelectionResetToken] =
    React.useState(0);

  const setTrackedWorkspace = React.useCallback<
    React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>
  >(
    (nextState) => {
      if (typeof nextState === "function") {
        const resolveNextState = nextState as (
          currentState: FlowStudioWorkspaceState,
        ) => FlowStudioWorkspaceState;
        const nextWorkspace = resolveNextState(workspaceRef.current);
        workspaceRef.current = nextWorkspace;
        options.setWorkspace(nextWorkspace);
        return;
      }

      workspaceRef.current = nextState;
      options.setWorkspace(nextState);
    },
    [options.setWorkspace],
  );

  const restoreState = React.useCallback(
    (historyState: DataflowUndoHistoryState) => {
      setTrackedWorkspace((currentWorkspace) =>
        restoreDataflowStructuralWorkspace({
          currentWorkspace,
          preserveLayout: !historyState.restoreLayout,
          snapshot: historyState.snapshot,
        }),
      );
      lastCapturedStateRef.current = historyState;
      setUndoSelectionResetToken((current) => current + 1);
    },
    [setTrackedWorkspace],
  );

  // Sync external snapshot drift into the most recent past entry's redo so
  // that redo replays the latest external state. Mirrors the in-house
  // useEffect that called `history.replacePresent` whenever the workspace
  // structure changed underneath the history. amend resolves to null when
  // the past stack is empty (no-op).
  React.useEffect(() => {
    const lastCaptured = lastCapturedStateRef.current;
    if (lastCaptured === null) {
      return;
    }

    const nextSnapshot = buildDataflowStructuralSnapshot(options.workspace);
    if (
      areDataflowStructuralSnapshotsExactlyEqual(
        lastCaptured.snapshot,
        nextSnapshot,
      )
    ) {
      return;
    }

    const updatedState: DataflowUndoHistoryState = {
      ...lastCaptured,
      snapshot: nextSnapshot,
    };
    lastCapturedStateRef.current = updatedState;
    void amnesia.amend({
      redo: () => {
        restoreState(updatedState);
      },
    });
  }, [amnesia, options.workspace.edges, options.workspace.execution, options.workspace.nodes, restoreState]);

  const captureSnapshot = React.useCallback(
    () => buildDataflowStructuralSnapshot(workspaceRef.current),
    [],
  );

  const commitLayoutChange = React.useCallback(
    (
      label: string,
      transform: (
        workspace: FlowStudioWorkspaceState,
      ) => FlowStudioWorkspaceState,
      changeOptions?: DataflowStructuralChangeOptions,
    ) => {
      const currentWorkspace = workspaceRef.current;
      const previousState =
        lastCapturedStateRef.current ??
        buildDataflowUndoHistoryState(
          buildDataflowStructuralSnapshot(currentWorkspace),
          false,
        );
      const nextWorkspace = commitDataflowSnapshotChange({
        amnesia,
        compareSnapshots: areDataflowStructuralSnapshotsExactlyEqual,
        label,
        options: {
          mergeKey: changeOptions?.mergeKey ?? null,
        },
        previousState,
        restoreLayout: true,
        restoreState,
        transform,
        workspace: currentWorkspace,
      });
      if (nextWorkspace === currentWorkspace) {
        return;
      }

      setTrackedWorkspace(nextWorkspace);
      lastCapturedStateRef.current = buildDataflowUndoHistoryState(
        buildDataflowStructuralSnapshot(nextWorkspace),
        true,
      );
    },
    [amnesia, restoreState, setTrackedWorkspace],
  );

  const commitStructuralChange = React.useCallback(
    (
      label: string,
      transform: (
        workspace: FlowStudioWorkspaceState,
      ) => FlowStudioWorkspaceState,
      changeOptions?: DataflowStructuralChangeOptions,
    ) => {
      const currentWorkspace = workspaceRef.current;
      const previousState =
        lastCapturedStateRef.current ??
        buildDataflowUndoHistoryState(
          buildDataflowStructuralSnapshot(currentWorkspace),
          false,
        );
      const nextWorkspace = commitDataflowSnapshotChange({
        amnesia,
        compareSnapshots: areDataflowStructuralSnapshotsEqual,
        label,
        options: {
          mergeKey: changeOptions?.mergeKey ?? null,
        },
        previousState,
        restoreLayout: false,
        restoreState,
        transform,
        workspace: currentWorkspace,
      });
      if (nextWorkspace === currentWorkspace) {
        return;
      }

      setTrackedWorkspace(nextWorkspace);
      lastCapturedStateRef.current = buildDataflowUndoHistoryState(
        buildDataflowStructuralSnapshot(nextWorkspace),
        false,
      );
    },
    [amnesia, restoreState, setTrackedWorkspace],
  );

  const pushPresentLayoutChange = React.useCallback(
    (
      label: string,
      previousSnapshot: DataflowStructuralSnapshot,
      changeOptions?: DataflowStructuralChangeOptions,
    ) => {
      const pushed = pushDataflowPresentSnapshotChange({
        amnesia,
        label,
        options: {
          mergeKey: changeOptions?.mergeKey ?? null,
        },
        previousSnapshot,
        restoreLayout: true,
        restoreState,
        workspace: workspaceRef.current,
      });
      if (!pushed) {
        return;
      }

      lastCapturedStateRef.current = buildDataflowUndoHistoryState(
        buildDataflowStructuralSnapshot(workspaceRef.current),
        true,
      );
    },
    [amnesia, restoreState],
  );

  const replaceWorkspaceAndResetUndo = React.useCallback(
    (nextWorkspace: FlowStudioWorkspaceState) => {
      setTrackedWorkspace(nextWorkspace);
      amnesia.clear();
      lastCapturedStateRef.current = buildDataflowUndoHistoryState(
        buildDataflowStructuralSnapshot(nextWorkspace),
        false,
      );
      setUndoSelectionResetToken((current) => current + 1);
    },
    [amnesia, setTrackedWorkspace],
  );

  return React.useMemo(
    () => ({
      amnesia,
      captureSnapshot,
      commitLayoutChange,
      commitStructuralChange,
      pushPresentLayoutChange,
      setTrackedWorkspace,
      replaceWorkspaceAndResetUndo,
      undoSelectionResetToken,
    }),
    [
      amnesia,
      captureSnapshot,
      commitLayoutChange,
      commitStructuralChange,
      pushPresentLayoutChange,
      setTrackedWorkspace,
      replaceWorkspaceAndResetUndo,
      undoSelectionResetToken,
    ],
  );
}
