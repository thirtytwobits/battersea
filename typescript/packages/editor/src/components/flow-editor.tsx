/** Copyright (c) Scott A Dixon */
import React from "react";
import { AmnesiaProvider } from "react-amnesia";
import type { FlowNodeDefinition } from "@battersea/flow";
import type {
  FlowEditorPorts,
  FlowLifecycleHost,
  FlowSummary,
  FlowDiagnostic,
  FlowActivation,
} from "../ports.js";
import type { FlowInteractionPorts } from "../interaction-ports.js";
import type { GraphPresentation } from "../graph.js";
import { loadLayoutEngine } from "../graph/layout/registry.js";
import {
  buildDefaultFlowWorkspace,
  buildFlowDocumentFromWorkspace,
  areFlowDocumentsEqual,
  findNextFlowNodeIndex,
} from "../core/flow-persistence.js";
import {
  createFlowDefinitionLookup,
  getPersistedParameters,
  getActivationParameters,
  resolveParameterEditorValue,
  formatFlowParameterLabel,
  nodeDefinitionIsActivatable,
} from "../core/flow-node-definitions.js";
import {
  createDroppedFlowNode,
  createFlowNodeDragPayload,
} from "../core/flow-drag.js";
import {
  buildRenderedFlowStudioNodes,
  findSelectedFlowStudioNode,
  findSelectedFlowStudioPort,
  resolveSelectedNodeDefinition,
} from "../core/dataflow-workspace-selectors.js";
import { updateWorkspaceNodeParameterValue } from "../core/dataflow-parameter-state.js";
import { useDataflowStructuralUndo } from "../hooks/use-dataflow-structural-undo.js";
import { useDataflowFlowLifecycle } from "../hooks/use-dataflow-flow-lifecycle.js";
import { useFlowCanvasEditing } from "../hooks/use-flow-canvas-editing.js";
import { useDataflowLayout } from "../hooks/use-dataflow-layout.js";
import { DataflowCanvas } from "./dataflow-canvas.js";
import { DataflowNodePalette } from "./dataflow-node-palette.js";
import { FlowInspector } from "./flow-inspector.js";
import {
  FlowParameterEditor,
  type FlowParameterRenderer,
} from "./flow-parameter-control.js";
export interface FlowEditorProps {
  ports: FlowEditorPorts;
  lifecycle: FlowLifecycleHost;
  interaction: FlowInteractionPorts;
  presentation?: GraphPresentation;
  parameterRenderers?: readonly FlowParameterRenderer[];
  runLayout?: import("../hooks/use-dataflow-layout.js").DataflowLayoutOptions["run"];
}
const localLayout: NonNullable<FlowEditorProps["runLayout"]> = async (
  id,
  graph,
  options,
) => (await loadLayoutEngine(id)).run(graph, options);
/** A complete unstyled editor. Hosts supply all I/O, prompts, notifications and specialised fields. */
export function FlowEditor(props: FlowEditorProps) {
  return (
    <AmnesiaProvider>
      <FlowEditorBody {...props} />
    </AmnesiaProvider>
  );
}
function FlowEditorBody({
  ports,
  lifecycle: host,
  interaction,
  presentation,
  parameterRenderers,
  runLayout = localLayout,
}: FlowEditorProps) {
  const [workspace, setWorkspace] = React.useState(buildDefaultFlowWorkspace);
  const [definitions, setDefinitions] = React.useState<
    readonly FlowNodeDefinition[]
  >([]);
  const [summaries, setSummaries] = React.useState<readonly FlowSummary[]>([]);
  const [error, setError] = React.useState("");
  const [name, setName] = React.useState("");
  const [activation, setActivation] = React.useState<FlowActivation | null>(
    null,
  );
  const [events, setEvents] = React.useState<FlowDiagnostic[]>([]);
  const [activationValues, setActivationValues] = React.useState<
    Record<string, unknown>
  >({});
  const [pending, setPending] = React.useState(false);
  const generation = React.useRef(0);
  const stop = React.useRef<(() => void) | null>(null);
  const mounted = React.useRef(true);
  React.useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      generation.current += 1;
      stop.current?.();
    };
  }, []);
  const reportError = React.useCallback(
    (error: unknown) => setError(String(error)),
    [],
  );
  const refresh = React.useCallback(async () => {
    const next = await ports.documents.list();
    setSummaries(next);
    return next;
  }, [ports]);
  React.useEffect(() => {
    let cancelled = false;
    void Promise.all([ports.catalogue.read(), ports.documents.list()])
      .then(([defs, list]) => {
        if (!cancelled) {
          setDefinitions(defs);
          setSummaries(list);
        }
      })
      .catch((error) => {
        if (!cancelled) reportError(error);
      });
    return () => {
      cancelled = true;
    };
  }, [ports, reportError]);
  const lookup = React.useMemo(
    () => createFlowDefinitionLookup([...definitions]),
    [definitions],
  );
  const undo = useDataflowStructuralUndo({ workspace, setWorkspace });
  const currentDocument = buildFlowDocumentFromWorkspace(workspace);
  const isDirty =
    !workspace.baselineFlow ||
    !areFlowDocumentsEqual(workspace.baselineFlow, currentDocument);
  const lifecycle = useDataflowFlowLifecycle({
    commitStructuralChange: undo.commitStructuralChange,
    host,
    documents: ports.documents,
    validation: ports.validation,
    isDirty,
    nodeDefinitions: definitions,
    replaceWorkspaceAndResetUndo: undo.replaceWorkspaceAndResetUndo,
    flowSummaries: summaries,
    refreshFlowSummaries: refresh,
    setWorkspace: undo.setTrackedWorkspace,
    workspace,
  });
  React.useEffect(
    () => setName(lifecycle.flowNameDialogState.initialValue),
    [
      lifecycle.flowNameDialogState.initialValue,
      lifecycle.flowNameDialogState.isOpen,
    ],
  );
  const selectedNode = findSelectedFlowStudioNode(workspace);
  const selectedDefinition = resolveSelectedNodeDefinition(
    selectedNode,
    lookup,
  );
  React.useEffect(
    () => setActivationValues({}),
    [selectedNode?.id, workspace.draftFlowKey],
  );
  const selectedPort = findSelectedFlowStudioPort(
    selectedNode,
    workspace.selectedTarget,
  );
  const canvas = useFlowCanvasEditing({
    ...undo,
    ports: interaction,
    workspace,
    setWorkspace: undo.setTrackedWorkspace,
    nodeDefinitionLookup: lookup,
    selectedNode,
    selectedNodeDefinition: selectedDefinition,
    selectedPort,
  });
  const autoLayout = useDataflowLayout({
    workspace,
    commit: undo.commitLayoutChange,
    run: runLayout,
    onError: reportError,
  });
  const activationParameters = selectedDefinition
    ? getActivationParameters(selectedDefinition)
    : [];
  const activate = async () => {
    if (!selectedNode || isDirty || pending) return;
    setPending(true);
    const attempt = ++generation.current;
    stop.current?.();
    setActivation(null);
    setError("");
    setEvents([]);
    try {
      const result = await ports.activation.activate({
        flow_key: workspace.draftFlowKey,
        node_id: selectedNode.id,
        parameters: Object.fromEntries(
          activationParameters.map((parameter) => [
            parameter.name,
            activationValues[parameter.name] ??
              (selectedDefinition
                ? resolveParameterEditorValue(
                    selectedDefinition,
                    parameter,
                    selectedNode.data.parameterValues,
                  )
                : undefined),
          ]),
        ),
      });
      if (!mounted.current || attempt !== generation.current) return;
      setActivation(result);
      stop.current = ports.activation.subscribe(result.id, (event) => {
        if (
          event.activation_id !== result.id ||
          !mounted.current ||
          attempt !== generation.current
        )
          return;
        setEvents((current) =>
          current.some((e) => e.sequence === event.sequence)
            ? current
            : [...current, event],
        );
        if (["succeeded", "failed", "cancelled"].includes(event.phase))
          setActivation({
            id: result.id,
            status: event.phase as FlowActivation["status"],
          });
      });
    } catch (error) {
      if (mounted.current) reportError(error);
    } finally {
      if (mounted.current) setPending(false);
    }
  };
  const renderedNodes = buildRenderedFlowStudioNodes(workspace);
  return (
    <div className="battersea-editor">
      <header className="battersea-editor__toolbar">
        <label>
          Flow
          <select
            aria-label="Flow"
            value={workspace.selectedFlowKey}
            onChange={(event) =>
              void lifecycle.handleSelectedFlowChange(event.target.value)
            }
          >
            <option value="">Choose a flow</option>
            {summaries.map((flow) => (
              <option
                key={flow.flow_key}
                value={flow.flow_key}
                disabled={Boolean(flow.unavailable_reason)}
              >
                {flow.title}
                {flow.unavailable_reason ? ` — ${flow.unavailable_reason}` : ""}
              </option>
            ))}
          </select>
        </label>
        <button onClick={() => void lifecycle.handleStartNewDraft()}>
          New
        </button>
        <button
          disabled={lifecycle.saving}
          onClick={() => void lifecycle.handleSaveCurrentFlow()}
        >
          Save
        </button>
        <button
          disabled={!lifecycle.refreshEnabled}
          onClick={() => void lifecycle.handleRefreshCurrentFlow()}
        >
          Reload
        </button>
        <button
          disabled={!lifecycle.cloneEnabled}
          onClick={() => lifecycle.openFlowNameDialog("clone")}
        >
          Clone
        </button>
        <button
          disabled={!lifecycle.deleteEnabled}
          onClick={() => void lifecycle.handleDeleteFlow()}
        >
          Delete
        </button>
        <button onClick={() => undo.amnesia.undo()}>Undo</button>
        <button onClick={() => undo.amnesia.redo()}>Redo</button>
        <output>{isDirty ? "Unsaved changes" : "Saved"}</output>
      </header>
      {lifecycle.flowNameDialogState.isOpen ? (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            lifecycle.handleFlowNameDialogConfirm(name);
          }}
        >
          <label>
            Flow name
            <input
              aria-label="Flow name"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </label>
          <button type="submit">Confirm name</button>
          <button type="button" onClick={lifecycle.closeFlowNameDialog}>
            Cancel
          </button>
        </form>
      ) : null}
      <div role="alert">{error || lifecycle.dataflowFieldState.message}</div>
      <div className="battersea-editor__workspace">
        <DataflowNodePalette
          nodeDefinitions={definitions}
          onAdd={(definition) =>
            undo.commitStructuralChange("Add node", (current) => {
              const node = createDroppedFlowNode({
                payload: createFlowNodeDragPayload(definition),
                nextIndex: findNextFlowNodeIndex(current.nodes),
                position: { x: 120, y: 120 },
              });
              return {
                ...current,
                nodes: [...current.nodes, node],
                selectedTarget: { kind: "node", nodeId: node.id },
              };
            })
          }
        />
        <div className="battersea-editor__canvas">
          <DataflowCanvas
            presentation={presentation}
            autoLayout={autoLayout}
            autoFitViewKey={lifecycle.autoFitViewKey}
            edgeActivationState={{}}
            edgeGestureDriver={canvas.edgeGestureDriver}
            edges={workspace.edges}
            nodes={renderedNodes}
            flowInstanceRef={canvas.flowInstanceRef}
            isConnectionValid={canvas.isConnectionValid}
            nodeContextMenu={canvas.nodeContextMenu}
            onConnect={canvas.handleConnect}
            onConnectEnd={canvas.handleConnectEnd}
            onEdgesChange={canvas.handleEdgesChange}
            onInsertEdgeBridge={canvas.handleInsertEdgeBridge}
            onInsertEdgeWaypoint={canvas.handleInsertEdgeWaypoint}
            onMovePort={canvas.handleMovePort}
            onNodeClick={canvas.handleNodeClick}
            onNodeContextMenu={canvas.handleNodeContextMenu}
            onNodeDragStart={canvas.handleNodeDragStart}
            onNodeDragStop={canvas.handleNodeDragStop}
            onNodesChange={canvas.handleNodesChange}
            onPaneClick={canvas.handlePaneClick}
            onPortSelect={canvas.handlePortSelect}
            onRemoveNode={canvas.removeNode}
            onSelectEdgeBridge={canvas.handleSelectEdgeBridge}
            onSelectEdgeWaypoint={canvas.handleSelectEdgeWaypoint}
            onUpdateEdgeBridgeGap={canvas.handleUpdateEdgeBridgeGap}
            onUpdateEdgeBridgePosition={canvas.handleUpdateEdgeBridgePosition}
            onUpdateEdgeWaypointHandle={canvas.handleUpdateEdgeWaypointHandle}
            onUpdateEdgeWaypointPosition={
              canvas.handleUpdateEdgeWaypointPosition
            }
            movedPortPulse={canvas.movedPortPulse}
            selectedEdgeBridge={canvas.selectedEdgeBridge}
            selectedEdgeWaypoint={canvas.selectedEdgeWaypoint}
            selectedPort={
              workspace.selectedTarget.kind === "port"
                ? workspace.selectedTarget
                : null
            }
          />
        </div>
        <aside aria-label="Inspector">
          <h2>{selectedNode?.data.instanceName ?? "Inspector"}</h2>
          {selectedNode && selectedDefinition ? (
            <>
              <FlowInspector
                parameters={getPersistedParameters(selectedDefinition)}
                values={selectedNode.data.parameterValues}
                renderers={parameterRenderers}
                onChange={(parameter, value) =>
                  undo.commitStructuralChange("Edit parameter", (current) =>
                    updateWorkspaceNodeParameterValue({
                      definition: selectedDefinition,
                      nodeId: selectedNode.id,
                      parameter,
                      value,
                      workspace: current,
                    }),
                  )
                }
              />
              {activationParameters.map((parameter) => (
                <FlowParameterEditor
                  key={parameter.name}
                  parameter={parameter}
                  fieldLabel={formatFlowParameterLabel(parameter.name)}
                  value={
                    activationValues[parameter.name] ??
                    resolveParameterEditorValue(
                      selectedDefinition,
                      parameter,
                      selectedNode.data.parameterValues,
                    )
                  }
                  onChange={(value) =>
                    setActivationValues((current) => ({
                      ...current,
                      [parameter.name]: value,
                    }))
                  }
                  renderers={parameterRenderers}
                />
              ))}
              <button
                disabled={
                  isDirty ||
                  pending ||
                  !nodeDefinitionIsActivatable(selectedDefinition) ||
                  activation?.status === "running"
                }
                onClick={() => void activate()}
              >
                Activate node
              </button>
            </>
          ) : null}
          <button
            disabled={activation?.status !== "running"}
            onClick={() => {
              if (activation)
                void ports.activation.cancel(activation.id).catch(reportError);
            }}
          >
            Cancel activation
          </button>
          <output aria-label="Activation status">
            {activation ? `${activation.id}: ${activation.status}` : "Idle"}
          </output>
          <ol aria-label="Diagnostics">
            {events.map((event) => (
              <li key={event.sequence} data-phase={event.phase}>
                {event.node_id}: {event.phase} {JSON.stringify(event.detail)}
              </li>
            ))}
          </ol>
        </aside>
      </div>
    </div>
  );
}
