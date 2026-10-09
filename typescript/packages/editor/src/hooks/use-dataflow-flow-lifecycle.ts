/**
 * Copyright (c) Scott A Dixon
 *
 * Manages dataflow flow lifecycle state, lifecycle, and derived behaviour for the editor's dataflow workspace.
 */
import React from "react";

import type { FlowNodeDefinition as WireFlowNodeDefinition } from "@battersea/flow";
import type {
  FlowSummary as WireFlowSummary,
  FlowDocumentPort,
  FlowEditorPorts,
  FlowLifecycleHost,
  FlowNameDialogState,
  DataflowDocumentFieldState,
  FlowNameModalMode,
} from "../ports.js";

import {
  buildDefaultFlowWorkspace,
  buildFlowSaveDocument,
  buildFlowValidationDocument,
  buildFlowWorkspaceFromDocument,
  renameNodeInstanceInWorkspace,
  slugifyFlowKey,
  type FlowStudioWorkspaceState,
} from "../core/flow-persistence.js";
import {
  buildWorkspaceForFailedFlowLoad,
  getFlowNameModalInitialValue,
  getLoadedFlowKey,
  getLoadedFlowTitle,
  hasLoadedNamedFlow,
  resolveFlowValidationMessage,
  resolveFlowSelectionChange,
  shouldPromptForFlowName,
} from "../core/flow-ui-state.js";
import type { DataflowStructuralChangeOptions } from "./use-dataflow-structural-undo.js";

export interface DataflowFlowLifecycleState {
  autoFitViewKey: string;
  closeFlowNameDialog: () => void;
  dataflowFieldState: DataflowDocumentFieldState;
  handleFlowNameDialogConfirm: (nextTitle: string) => void;
  handleDeleteFlow: () => Promise<void>;
  handleRefreshCurrentFlow: () => Promise<void>;
  handleSaveCurrentFlow: () => Promise<void>;
  handleSelectedFlowChange: (nextFlowKey: string) => Promise<void>;
  handleStartNewDraft: () => Promise<boolean>;
  cloneEnabled: boolean;
  cloneSuccessPunchToken: number;
  isDirty: boolean;
  loadedFlowKey: string;
  loadedFlowTitle: string;
  loadingFlow: boolean;
  openFlowNameDialog: (mode: FlowNameModalMode) => void;
  openNodeRenameDialog: (nodeId: string, instanceName: string) => void;
  deleteEnabled: boolean;
  deleteSuccessPunchToken: number;
  refreshEnabled: boolean;
  refreshSuccessPunchToken: number;
  renameEnabled: boolean;
  renameSuccessPunchToken: number;
  saveSuccessPunchToken: number;
  saving: boolean;
  flowNameDialogState: FlowNameDialogState;
}

export function useDataflowFlowLifecycle(options: {
  commitStructuralChange: (
    label: string,
    transform: (
      workspace: FlowStudioWorkspaceState,
    ) => FlowStudioWorkspaceState,
    options?: DataflowStructuralChangeOptions,
  ) => void;
  host: FlowLifecycleHost;
  documents: FlowDocumentPort;
  validation: FlowEditorPorts["validation"];
  isDirty: boolean;
  nodeDefinitions: readonly WireFlowNodeDefinition[];
  replaceWorkspaceAndResetUndo: (
    nextWorkspace: FlowStudioWorkspaceState,
  ) => void;
  flowSummaries: readonly WireFlowSummary[];
  refreshFlowSummaries: () => Promise<readonly WireFlowSummary[]>;
  setWorkspace: React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>;
  workspace: FlowStudioWorkspaceState;
}): DataflowFlowLifecycleState {
  const [loadingFlow, setLoadingFlow] = React.useState(false);
  const [saving, setSaving] = React.useState(false);
  const [dataflowFieldState, setDataflowFieldState] =
    React.useState<DataflowDocumentFieldState>({
      message: "",
      status: "idle",
    });
  const [flowNameDialogState, setFlowNameDialogState] =
    React.useState<FlowNameDialogState>({
      initialValue: "",
      isOpen: false,
      mode: "name",
      pendingAction: null,
      targetFlowKey: null,
      targetNodeId: null,
    });
  const [saveSuccessPunchToken, setSaveSuccessPunchToken] = React.useState(0);
  const [cloneSuccessPunchToken, setCloneSuccessPunchToken] = React.useState(0);
  const [renameSuccessPunchToken, setRenameSuccessPunchToken] =
    React.useState(0);
  const [deleteSuccessPunchToken, setDeleteSuccessPunchToken] =
    React.useState(0);
  const [refreshSuccessPunchToken, setRefreshSuccessPunchToken] =
    React.useState(0);
  const [autoFitViewKey, setAutoFitViewKey] = React.useState(() =>
    getLoadedFlowKey(options.workspace),
  );

  const loadedFlowKey = getLoadedFlowKey(options.workspace);
  const loadedFlowTitle = getLoadedFlowTitle(options.workspace);
  const renameEnabled = hasLoadedNamedFlow(options.workspace);
  const deleteEnabled = hasLoadedNamedFlow(options.workspace);
  const refreshEnabled = hasLoadedNamedFlow(options.workspace);
  const cloneEnabled =
    hasLoadedNamedFlow(options.workspace) && !options.isDirty;

  const clearDataflowFieldState = React.useCallback(() => {
    setDataflowFieldState({
      message: "",
      status: "idle",
    });
  }, []);

  const openFlowNameDialog = React.useCallback(
    (mode: FlowNameModalMode) => {
      setFlowNameDialogState({
        initialValue: getFlowNameModalInitialValue(
          mode,
          loadedFlowTitle,
          options.flowSummaries,
        ),
        isOpen: true,
        mode,
        pendingAction:
          mode === "clone" ? "clone" : mode === "rename" ? "rename" : "save",
        targetFlowKey: null,
        targetNodeId: null,
      });
    },
    [loadedFlowTitle, options.flowSummaries],
  );

  const openNodeRenameDialog = React.useCallback(
    (nodeId: string, instanceName: string) => {
      setFlowNameDialogState({
        initialValue: instanceName,
        isOpen: true,
        mode: "rename-node",
        pendingAction: "rename-node",
        targetFlowKey: options.workspace.draftFlowKey,
        targetNodeId: nodeId,
      });
    },
    [options.workspace.draftFlowKey],
  );

  const closeFlowNameDialog = React.useCallback(() => {
    setFlowNameDialogState({
      initialValue: "",
      isOpen: false,
      mode: "name",
      pendingAction: null,
      targetFlowKey: null,
      targetNodeId: null,
    });
  }, []);

  const persistCurrentFlow = React.useCallback(
    async (
      titleOverride?: string,
      successAction: "rename" | "save" = "save",
    ): Promise<boolean> => {
      const flowToSave = buildFlowSaveDocument({
        titleOverride,
        workspace: options.workspace,
      });
      if (!flowToSave) {
        return false;
      }

      setSaving(true);

      try {
        const saveResult = await options.documents.save(flowToSave);

        options.setWorkspace((current) => ({
          ...current,
          baselineFlow: saveResult.flow,
          draftFlowKey: saveResult.flow.flow_key,
          selectedFlowKey: saveResult.flow.flow_key,
          title: saveResult.flow.title,
          description:
            saveResult.flow.description?.trim() ?? current.description,
        }));
        clearDataflowFieldState();
        await options.refreshFlowSummaries();
        if (successAction === "rename") {
          setRenameSuccessPunchToken((current) => current + 1);
        } else {
          setSaveSuccessPunchToken((current) => current + 1);
        }
        return true;
      } catch (error) {
        console.error("Could not save the current flow.", error);
        options.host.notify({
          id: "flow-save-error",
          tone: "error",
          title: "Save failed",
          message:
            error instanceof Error
              ? error.message
              : "The dataflow could not be saved.",
        });
        return false;
      } finally {
        setSaving(false);
      }
    },
    [
      clearDataflowFieldState,
      options.documents,
      options.validation,
      options.host,
      options.refreshFlowSummaries,
      options.setWorkspace,
      options.workspace.description,
      options.workspace.draftFlowKey,
      options.workspace.edges,
      options.workspace.nodes,
      options.workspace.title,
    ],
  );

  const validateCurrentFlow = React.useCallback(async (): Promise<boolean> => {
    const validationTarget = buildFlowValidationDocument({
      workspace: options.workspace,
    });

    try {
      const validation = await options.validation.validate(validationTarget);
      if (validation.valid) {
        clearDataflowFieldState();
        return true;
      }

      const message = resolveFlowValidationMessage(
        validation.issues[0]?.message ?? "The dataflow is invalid.",
      );
      setDataflowFieldState({
        message,
        status: "error",
      });
      options.host.notify({
        id: "flow-save-error",
        tone: "error",
        title: "Save failed",
        message,
      });
      return false;
    } catch (error) {
      console.error("Could not validate the current flow.", error);
      const message = resolveFlowValidationMessage(
        error instanceof Error
          ? error.message
          : "The dataflow could not be validated.",
      );
      setDataflowFieldState({
        message,
        status: "error",
      });
      options.host.notify({
        id: "flow-validate-error",
        tone: "error",
        title: "Validation failed",
        message,
      });
      return false;
    }
  }, [
    clearDataflowFieldState,
    options.documents,
    options.validation,
    options.host,
    options.workspace,
    resolveFlowValidationMessage,
  ]);

  const cloneCurrentFlow = React.useCallback(
    async (nextTitle: string): Promise<boolean> => {
      const currentFlowKey = loadedFlowKey.trim();
      const title = nextTitle.trim();
      if (!currentFlowKey || !title) {
        return false;
      }

      setSaving(true);

      try {
        const cloneResult = await options.documents.clone({
          current_flow_key: currentFlowKey,
          next_flow_key: slugifyFlowKey(title),
          next_title: title,
        });

        setAutoFitViewKey(cloneResult.flow.flow_key);
        options.replaceWorkspaceAndResetUndo(
          buildFlowWorkspaceFromDocument({
            definitions: [...options.nodeDefinitions],
            document: cloneResult.flow,
          }),
        );
        clearDataflowFieldState();
        await options.refreshFlowSummaries();
        setCloneSuccessPunchToken((current) => current + 1);
        return true;
      } catch (error) {
        console.error(`Could not clone flow "${currentFlowKey}".`, error);
        options.host.notify({
          id: `flow-clone-error:${currentFlowKey}`,
          tone: "error",
          title: "Clone failed",
          message:
            error instanceof Error
              ? error.message
              : "The dataflow could not be cloned.",
        });
        return false;
      } finally {
        setSaving(false);
      }
    },
    [
      clearDataflowFieldState,
      loadedFlowKey,
      options.documents,
      options.validation,
      options.host,
      options.nodeDefinitions,
      options.refreshFlowSummaries,
      options.replaceWorkspaceAndResetUndo,
    ],
  );

  const loadFlow = React.useCallback(
    async (flowKey: string): Promise<boolean> => {
      const nextFlowKey = flowKey.trim();
      if (!nextFlowKey || options.nodeDefinitions.length === 0) {
        return false;
      }

      setAutoFitViewKey(nextFlowKey);
      setLoadingFlow(true);
      setDataflowFieldState({
        message: "",
        status: "loading",
      });

      try {
        const document = await options.documents.read(nextFlowKey);

        setAutoFitViewKey(document.flow_key);
        options.replaceWorkspaceAndResetUndo(
          buildFlowWorkspaceFromDocument({
            document,
            definitions: [...options.nodeDefinitions],
          }),
        );
        clearDataflowFieldState();
        return true;
      } catch (error) {
        console.error(`Could not load flow "${nextFlowKey}".`, error);
        const message =
          error instanceof Error
            ? error.message
            : "The selected dataflow could not be loaded.";
        setAutoFitViewKey("");
        options.replaceWorkspaceAndResetUndo(buildWorkspaceForFailedFlowLoad());
        setDataflowFieldState({
          message,
          status: "error",
        });
        options.host.notify({
          id: `flow-load-error:${nextFlowKey}`,
          tone: "error",
          title: "Load failed",
          message,
        });
        return false;
      } finally {
        setLoadingFlow(false);
      }
    },
    [
      clearDataflowFieldState,
      options.documents,
      options.validation,
      options.host,
      options.nodeDefinitions,
      options.replaceWorkspaceAndResetUndo,
    ],
  );

  const confirmDataflowDiscard = React.useCallback(
    async (confirmOptions: {
      actionLabel: string;
      message: string;
      title: string;
    }): Promise<boolean> => {
      if (!options.isDirty) {
        return true;
      }

      return options.host.confirm(confirmOptions);
    },
    [options.isDirty, options.host],
  );

  const confirmDiscardUnsavedWork = React.useCallback(
    async (message: string, actionLabel: string): Promise<boolean> => {
      return options.host.confirmDiscard({
        actionLabel,
        message,
        title: "Discard unsaved work?",
      });
    },
    [options.host],
  );

  const handleStartNewDraft = React.useCallback(async (): Promise<boolean> => {
    const discard = await confirmDiscardUnsavedWork(
      "Starting a new dataflow will discard your unsaved changes.",
      "Start new draft",
    );

    if (!discard) {
      return false;
    }

    clearDataflowFieldState();
    options.replaceWorkspaceAndResetUndo(buildDefaultFlowWorkspace());
    setAutoFitViewKey("");
    return true;
  }, [
    clearDataflowFieldState,
    confirmDiscardUnsavedWork,
    options.replaceWorkspaceAndResetUndo,
  ]);

  const handleSaveCurrentFlow = React.useCallback(async (): Promise<void> => {
    const valid = await validateCurrentFlow();
    if (!valid) {
      return;
    }

    if (shouldPromptForFlowName(options.workspace.title)) {
      openFlowNameDialog("name");
      return;
    }

    await persistCurrentFlow();
  }, [
    openFlowNameDialog,
    persistCurrentFlow,
    options.workspace.title,
    validateCurrentFlow,
  ]);

  const handleDeleteFlow = React.useCallback(async (): Promise<void> => {
    if (!loadedFlowKey.trim()) {
      return;
    }

    const confirmed = await options.host.confirm({
      actionLabel: "Delete dataflow",
      cancelLabel: "Keep dataflow",
      message: options.host.deleteConfirmation(
        loadedFlowTitle || options.workspace.title || loadedFlowKey,
        options.isDirty,
      ),
      title: "Delete dataflow?",
    });
    if (!confirmed) {
      return;
    }

    setSaving(true);

    try {
      await options.documents.delete({
        flow_key: loadedFlowKey,
      });
      const remainingFlows = await options.refreshFlowSummaries();

      if (remainingFlows.length === 0) {
        clearDataflowFieldState();
        options.replaceWorkspaceAndResetUndo(buildDefaultFlowWorkspace());
        return;
      }

      const nextFlowSummary = remainingFlows[0];
      await loadFlow(nextFlowSummary.flow_key);
      setDeleteSuccessPunchToken((current) => current + 1);
    } catch (error) {
      console.error(`Could not delete flow "${loadedFlowKey}".`, error);
      options.host.notify({
        id: `flow-delete-error:${loadedFlowKey}`,
        tone: "error",
        title: "Delete failed",
        message:
          error instanceof Error
            ? error.message
            : "The dataflow could not be deleted.",
      });
    } finally {
      setSaving(false);
    }
  }, [
    clearDataflowFieldState,
    loadFlow,
    loadedFlowKey,
    loadedFlowTitle,
    options.documents,
    options.validation,
    options.host,
    options.isDirty,
    options.refreshFlowSummaries,
    options.replaceWorkspaceAndResetUndo,
    options.workspace.title,
  ]);

  React.useEffect(() =>
    options.host.registerDirty({
      canSave: !saving,
      confirmDiscard: confirmDataflowDiscard,
      isDirty: options.isDirty,
      save: handleSaveCurrentFlow,
      saveLabel: "Save dataflow",
      saveTitle: options.isDirty ? "Unsaved changes" : "Save dataflow",
    }),
  );
  const hostRef = React.useRef(options.host);
  hostRef.current = options.host;
  React.useEffect(() => () => hostRef.current.unregisterDirty(), []);

  const handleSelectedFlowChange = React.useCallback(
    async (nextFlowKey: string): Promise<void> => {
      clearDataflowFieldState();
      const previousLoadedFlowKey = loadedFlowKey;

      options.setWorkspace((current) => ({
        ...current,
        selectedFlowKey: nextFlowKey,
      }));

      if (!nextFlowKey) {
        const started = await handleStartNewDraft();
        if (!started) {
          options.setWorkspace((current) => ({
            ...current,
            selectedFlowKey: previousLoadedFlowKey,
          }));
        }
        return;
      }

      const result = await resolveFlowSelectionChange({
        currentLoadedFlowKey: previousLoadedFlowKey,
        isDirty: options.isDirty,
        loadFlow,
        nextFlowKey,
        confirmDiscard: () =>
          confirmDiscardUnsavedWork(
            "Loading another dataflow now will discard your unsaved changes.",
            "Discard and Load dataflow",
          ),
      });

      if (result === "cancelled") {
        options.setWorkspace((current) => ({
          ...current,
          selectedFlowKey: previousLoadedFlowKey,
        }));
      }
    },
    [
      clearDataflowFieldState,
      confirmDiscardUnsavedWork,
      handleStartNewDraft,
      loadFlow,
      loadedFlowKey,
      options.isDirty,
      options.setWorkspace,
    ],
  );

  const handleRefreshCurrentFlow =
    React.useCallback(async (): Promise<void> => {
      if (!loadedFlowKey.trim()) {
        return;
      }

      const discard = await confirmDiscardUnsavedWork(
        "Refreshing reloads this dataflow from the engine and discards your unsaved changes.",
        "Discard and refresh dataflow",
      );
      if (!discard) {
        return;
      }

      const loaded = await loadFlow(loadedFlowKey);
      if (loaded) {
        setRefreshSuccessPunchToken((current) => current + 1);
      }
    }, [confirmDiscardUnsavedWork, loadFlow, loadedFlowKey]);

  const handleFlowNameDialogConfirm = React.useCallback(
    (nextTitle: string) => {
      const trimmedTitle = nextTitle.trim();
      if (!trimmedTitle) {
        return;
      }

      const pendingAction = flowNameDialogState.pendingAction;
      const targetNodeId = flowNameDialogState.targetNodeId;

      if (pendingAction === "rename-node") {
        if (!targetNodeId) {
          return;
        }

        setFlowNameDialogState((current) => ({
          ...current,
          isOpen: false,
          pendingAction: null,
          targetFlowKey: null,
          targetNodeId: null,
        }));
        options.commitStructuralChange(
          "Rename node",
          (current) =>
            current.draftFlowKey === flowNameDialogState.targetFlowKey
              ? renameNodeInstanceInWorkspace(
                  current,
                  targetNodeId,
                  trimmedTitle,
                )
              : current,
          {
            mergeKey: JSON.stringify([
              "node-rename",
              flowNameDialogState.targetFlowKey,
              targetNodeId,
            ]),
          },
        );
        return;
      }

      if (pendingAction === "clone") {
        void (async () => {
          const cloned = await cloneCurrentFlow(trimmedTitle);
          if (cloned) {
            closeFlowNameDialog();
          }
        })();
        return;
      }

      if (!pendingAction) {
        return;
      }

      void (async () => {
        const saved = await persistCurrentFlow(
          trimmedTitle,
          pendingAction === "rename" ? "rename" : "save",
        );
        if (saved) {
          closeFlowNameDialog();
        }
      })();
    },
    [
      cloneCurrentFlow,
      options.commitStructuralChange,
      closeFlowNameDialog,
      flowNameDialogState.pendingAction,
      flowNameDialogState.targetNodeId,
      flowNameDialogState.targetFlowKey,
      persistCurrentFlow,
    ],
  );

  return {
    autoFitViewKey,
    cloneEnabled,
    cloneSuccessPunchToken,
    closeFlowNameDialog,
    dataflowFieldState,
    deleteEnabled,
    deleteSuccessPunchToken,
    flowNameDialogState,
    handleDeleteFlow,
    handleFlowNameDialogConfirm,
    handleRefreshCurrentFlow,
    handleSaveCurrentFlow,
    handleSelectedFlowChange,
    handleStartNewDraft,
    isDirty: options.isDirty,
    loadedFlowKey,
    loadedFlowTitle,
    loadingFlow,
    openFlowNameDialog,
    openNodeRenameDialog,
    refreshEnabled,
    refreshSuccessPunchToken,
    renameEnabled,
    renameSuccessPunchToken,
    saveSuccessPunchToken,
    saving,
  };
}
