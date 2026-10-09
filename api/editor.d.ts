// components/dataflow-canvas.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Renders the dataflow canvas component for the editor's dataflow workspace.
 */
import React from "react";
import type { GraphPresentation } from "../graph.js";
import { type Connection, type EdgeChange, type FinalConnectionState, type NodeChange, type OnNodeDrag, type ReactFlowInstance } from "@xyflow/react";
import type { AuthoringGraphInteractiveGestureDriver } from "../graph.js";
import type { FlowStudioEdge, FlowStudioEdgeBridge, FlowStudioNode, FlowStudioPoint, FlowStudioPortSelectionTarget, FlowStudioSelectedEdgeBridge, FlowStudioSelectedEdgeWaypoint, FlowStudioEdgeWaypoint, NodeContextMenuState } from "../core/dataflow-editor-state.js";
import type { FlowStudioEdgeActivationState } from "../core/flow-edge-activation.js";
import type { FlowPortSide } from "../core/flow-node-ports.js";
export interface DataflowCanvasProps {
    presentation?: GraphPresentation;
    autoLayout?: import("../graph.js").AuthoringGraphAutoLayoutController;
    autoFitViewKey?: string;
    edgeActivationState: FlowStudioEdgeActivationState;
    edges: readonly FlowStudioEdge[];
    flowInstanceRef: React.MutableRefObject<ReactFlowInstance<FlowStudioNode, FlowStudioEdge> | null>;
    edgeGestureDriver: AuthoringGraphInteractiveGestureDriver;
    isConnectionValid: (connection: Connection | FlowStudioEdge) => boolean;
    nodeContextMenu: NodeContextMenuState | null;
    nodes: readonly FlowStudioNode[];
    onConnect: (connection: Connection) => void;
    onConnectEnd: (event: MouseEvent | TouchEvent, connectionState: FinalConnectionState) => void;
    onEdgesChange: (changes: EdgeChange<FlowStudioEdge>[]) => void;
    onNodeClick: (_event: React.MouseEvent, node: FlowStudioNode) => void;
    onNodeContextMenu: (event: React.MouseEvent, node: FlowStudioNode) => void;
    onNodesChange: (changes: NodeChange<FlowStudioNode>[]) => void;
    onPaneClick: () => void;
    onInsertEdgeBridge: (edgeId: string, segmentIndex: number, bridge: FlowStudioEdgeBridge) => void;
    onInsertEdgeWaypoint: (edgeId: string, segmentIndex: number, segmentT: number, waypoint: FlowStudioEdgeWaypoint) => void;
    onMovePort: (target: {
        direction: "toward-end" | "toward-start";
        nodeId: string;
        portId: string;
        side: FlowPortSide;
    }) => void;
    onPortSelect?: (target: {
        nodeId: string;
        portId: string;
        side: FlowStudioPortSelectionTarget["side"];
    }) => void;
    onNodeDragStart: OnNodeDrag<FlowStudioNode>;
    onNodeDragStop: OnNodeDrag<FlowStudioNode>;
    onRemoveNode: (nodeId: string) => void;
    onSelectEdgeBridge: (edgeId: string, bridgeIndex: number) => void;
    onSelectEdgeWaypoint: (edgeId: string, waypointIndex: number) => void;
    onUpdateEdgeBridgeGap: (edgeId: string, bridgeIndex: number, gap: number) => void;
    onUpdateEdgeBridgePosition: (edgeId: string, bridgeIndex: number, segmentIndex: number, t: number) => void;
    onUpdateEdgeWaypointHandle: (edgeId: string, waypointIndex: number, handleKind: "inHandle" | "outHandle", independent: boolean, position: FlowStudioPoint) => void;
    onUpdateEdgeWaypointPosition: (edgeId: string, waypointIndex: number, position: FlowStudioPoint) => void;
    movedPortPulse?: {
        nodeId: string;
        portId: string;
        replay: "a" | "b";
        side: FlowPortSide;
    } | null;
    selectedEdgeBridge?: FlowStudioSelectedEdgeBridge | null;
    selectedEdgeWaypoint?: FlowStudioSelectedEdgeWaypoint | null;
    selectedPort?: FlowStudioPortSelectionTarget | null;
}
export declare function DataflowCanvas({ autoLayout, presentation, autoFitViewKey, edgeActivationState, edgeGestureDriver, edges, flowInstanceRef, isConnectionValid, nodeContextMenu, nodes, onConnect, onConnectEnd, onEdgesChange, onNodeClick, onNodeContextMenu, onNodesChange, onPaneClick, onInsertEdgeBridge, onInsertEdgeWaypoint, onMovePort, onPortSelect, onNodeDragStart, onNodeDragStop, onRemoveNode, onSelectEdgeBridge, onSelectEdgeWaypoint, onUpdateEdgeBridgeGap, onUpdateEdgeBridgePosition, onUpdateEdgeWaypointHandle, onUpdateEdgeWaypointPosition, movedPortPulse, selectedEdgeBridge, selectedEdgeWaypoint, selectedPort, }: DataflowCanvasProps): React.JSX.Element;

// components/dataflow-node-palette.d.ts
/** Copyright (c) Scott A Dixon */
import React from "react";
import type { FlowNodeDefinition } from "@battersea/flow";
import { buildDataflowNodePaletteGroups, type DataflowNodePaletteEntry } from "../core/dataflow-node-palette-state.js";
export interface FlowPaletteModel {
    groups: ReturnType<typeof buildDataflowNodePaletteGroups>;
    dragId: (entry: DataflowNodePaletteEntry) => string;
    dragData: (entry: DataflowNodePaletteEntry) => {
        flowNodeDragPayload: string;
        nodeDefinition: FlowNodeDefinition;
    };
}
export declare function DataflowNodePalette({ nodeDefinitions, render, onAdd, }: {
    nodeDefinitions: readonly FlowNodeDefinition[];
    render?: (model: FlowPaletteModel) => React.ReactNode;
    onAdd?: (definition: FlowNodeDefinition) => void;
}): React.JSX.Element;

// components/flow-editor.d.ts
/** Copyright (c) Scott A Dixon */
import React from "react";
import type { FlowEditorPorts, FlowLifecycleHost } from "../ports.js";
import type { FlowInteractionPorts } from "../interaction-ports.js";
import type { GraphPresentation } from "../graph.js";
import { type FlowParameterRenderer } from "./flow-parameter-control.js";
export interface FlowEditorProps {
    ports: FlowEditorPorts;
    lifecycle: FlowLifecycleHost;
    interaction: FlowInteractionPorts;
    presentation?: GraphPresentation;
    parameterRenderers?: readonly FlowParameterRenderer[];
    runLayout?: import("../hooks/use-dataflow-layout.js").DataflowLayoutOptions["run"];
}
/** A complete unstyled editor. Hosts supply all I/O, prompts, notifications and specialised fields. */
export declare function FlowEditor(props: FlowEditorProps): React.JSX.Element;

// components/flow-inspector.d.ts
/** Copyright (c) Scott A Dixon */
import React from "react";
import type { FlowParameterDefinition } from "@battersea/flow";
import { groupControlsForDetailPane } from "../core/flow-control-layout.js";
import { type FlowParameterRenderer } from "./flow-parameter-control.js";
type Group = ReturnType<typeof groupControlsForDetailPane>[number];
export interface FlowInspectorProps {
    parameters: readonly FlowParameterDefinition[];
    values?: Readonly<Record<string, unknown>>;
    onChange?: (parameter: FlowParameterDefinition, value: unknown) => void;
    renderers?: readonly FlowParameterRenderer[];
    renderGroup?: (group: Group) => React.ReactNode | undefined;
    renderParameter?: (parameter: FlowParameterDefinition, group: Group) => React.ReactNode;
}
/** Grouping and renderer selection are shared; hosts own product-specific sections and framing. */
export declare function FlowInspector({ parameters, values, onChange, renderers, renderGroup, renderParameter, }: FlowInspectorProps): React.JSX.Element;
export {};

// components/flow-parameter-control.d.ts
/** Copyright (c) Scott A Dixon */
import React from "react";
import type { FlowParameterDefinition } from "@battersea/flow";
export interface ParameterOption {
    label: React.ReactNode;
    value: string;
    disabled?: boolean;
}
interface ChoiceProps {
    ariaLabel: string;
    disabled?: boolean;
    fullWidth?: boolean;
    minSegmentWidthPx?: number;
    onChange: (value: string) => void;
    options: readonly [ParameterOption, ...ParameterOption[]];
    value: string;
    wrap?: boolean;
}
export interface ParameterPresentation {
    Field: React.ComponentType<{
        as?: "label";
        className?: string;
        compact?: boolean;
        label?: React.ReactNode;
        children: React.ReactNode;
    }>;
    Select: React.ComponentType<React.SelectHTMLAttributes<HTMLSelectElement>>;
    Switch: React.ComponentType<Omit<ChoiceProps, "options"> & {
        options: [ParameterOption, ParameterOption];
    }>;
    Segments: React.ComponentType<ChoiceProps>;
}
export declare const nativeParameterPresentation: ParameterPresentation;
export interface FlowParameterControlProps {
    parameter: FlowParameterDefinition;
    value: unknown;
    onChange: (value: unknown) => void;
    options?: readonly ParameterOption[];
    presentation?: ParameterPresentation;
    compact?: boolean;
    heading?: string;
    fieldLabel?: React.ReactNode;
    disabled?: boolean;
    help?: React.ReactNode;
    onUnsupportedInteraction?: () => void;
}
/** Custom source editors are supplied by the host's renderer before this primitive control. */
export declare function FlowParameterControl({ parameter, value, onChange, options, presentation, compact, heading, fieldLabel, disabled, help, onUnsupportedInteraction, }: FlowParameterControlProps): React.JSX.Element;
export type FlowParameterRenderer = (props: FlowParameterControlProps) => React.ReactNode | undefined;
/** Undefined delegates to the built-in control; null intentionally hides a host-rendered field. */
export declare function FlowParameterEditor({ renderers, ...props }: FlowParameterControlProps & {
    renderers?: readonly FlowParameterRenderer[];
}): React.JSX.Element;
export {};

// core/dataflow-cardinality.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Implements dataflow cardinality for the editor's dataflow workspace.
 */
export declare function formatCardinalityRange(min: number, max: number | null): string;
export declare function isFixedCardinality(min: number, max: number | null): boolean;
export declare function buildCardinalityOptions(min: number, max: number): number[];
export declare function resolveCardinalityValue(value: unknown, min: number): number;

// core/dataflow-edge-interactions.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Resolves selection and layout updates for interactive Dataflow edge affordances.
 */
import type { FlowStudioEdgeBridge, FlowStudioEdgeWaypoint, FlowStudioSelectedEdgeBridge, FlowStudioSelectedEdgeWaypoint } from "./dataflow-editor-state.js";
import { type FlowStudioWorkspaceState } from "./flow-persistence.js";
export interface DataflowEdgeInteractionResult {
    selectedEdgeBridge: FlowStudioSelectedEdgeBridge | null;
    selectedEdgeWaypoint: FlowStudioSelectedEdgeWaypoint | null;
    workspace: FlowStudioWorkspaceState;
}
export declare function clearNativeDataflowEdgeSelection(workspace: FlowStudioWorkspaceState): FlowStudioWorkspaceState;
export declare function resolveDataflowEdgeBridgeSelection(workspace: FlowStudioWorkspaceState, edgeId: string, bridgeIndex: number): DataflowEdgeInteractionResult;
export declare function resolveDataflowEdgeWaypointSelection(workspace: FlowStudioWorkspaceState, edgeId: string, waypointIndex: number): DataflowEdgeInteractionResult;
export declare function resolveDataflowEdgeBridgeInsertion(workspace: FlowStudioWorkspaceState, edgeId: string, bridge: FlowStudioEdgeBridge): DataflowEdgeInteractionResult;
export declare function resolveDataflowEdgeWaypointInsertion(workspace: FlowStudioWorkspaceState, edgeId: string, segmentIndex: number, segmentT: number, waypoint: FlowStudioEdgeWaypoint): DataflowEdgeInteractionResult;
export declare function updateDataflowEdgeBridgePosition(workspace: FlowStudioWorkspaceState, edgeId: string, bridgeIndex: number, segmentIndex: number, t: number): FlowStudioWorkspaceState;
export declare function updateDataflowEdgeBridgeGap(workspace: FlowStudioWorkspaceState, edgeId: string, bridgeIndex: number, gap: number): FlowStudioWorkspaceState;

// core/dataflow-editor-state.d.ts
/** Copyright (c) Scott A Dixon */
import type { Edge, Node } from "@xyflow/react";
import type { FlowEdge as WireFlowEdge } from "@battersea/flow";
import type { AuthoringEditableEdgeData, AuthoringGraphBridge, AuthoringGraphPoint, AuthoringGraphWaypoint } from "../graph.js";
import type { FlowStudioNodeData } from "./flow-drag.js";
import type { FlowPortSide } from "./flow-node-ports.js";
export interface NodeContextMenuState {
    nodeId: string;
    x: number;
    y: number;
}
export interface PendingNodeDropState {
    status: "idle" | "pending" | "failed" | "succeeded";
    title: string;
}
export type FlowStudioPoint = AuthoringGraphPoint;
export type FlowStudioEdgeWaypoint = AuthoringGraphWaypoint;
export type FlowStudioEdgeBridge = AuthoringGraphBridge;
export interface FlowStudioSelectedEdgeWaypoint {
    edgeId: string;
    waypointIndex: number;
}
export interface FlowStudioSelectedEdgeBridge {
    bridgeIndex: number;
    edgeId: string;
}
export type FlowStudioEdgeData = AuthoringEditableEdgeData<{
    edgeClassName?: string;
    kind: WireFlowEdge["kind"];
    order: number;
    queue?: WireFlowEdge["queue"];
    sourceHandleIndex?: number;
    sourceSideCount?: number;
    targetHandleIndex?: number;
    targetSideCount?: number;
    tokenType?: string;
}>;
export interface FlowStudioNodeSelectionTarget {
    kind: "node";
    nodeId: string;
}
export interface FlowStudioNoSelectionTarget {
    kind: "none";
}
export interface FlowStudioFlowSelectionTarget {
    kind: "flow";
}
export interface FlowStudioPortSelectionTarget {
    kind: "port";
    nodeId: string;
    portId: string;
    side: FlowPortSide;
}
export type FlowStudioSelectionTarget = FlowStudioNoSelectionTarget | FlowStudioFlowSelectionTarget | FlowStudioNodeSelectionTarget | FlowStudioPortSelectionTarget;
export type FlowStudioNode = Node<FlowStudioNodeData>;
export type FlowStudioEdge = Edge<FlowStudioEdgeData>;

// core/dataflow-node-palette-state.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Shapes grouped palette data for the dataflow workspace.
 */
import type { FlowNodeClass, FlowNodeDefinition } from "@battersea/flow";
interface AuthoringPaletteEntryBase {
    badges?: readonly {
        classifier: FlowNodeClass | "controller";
        icon?: string;
        label: string;
        shape?: "icon" | "pill";
        title?: string;
    }[];
    description: string;
    groupId: string;
    groupLabel: string;
    title: string;
    variant: string;
}
export interface DataflowNodePaletteEntry extends AuthoringPaletteEntryBase {
    definition: FlowNodeDefinition;
}
export declare function buildDataflowNodePaletteGroups(nodeDefinitions: readonly FlowNodeDefinition[]): {
    id: string;
    label: string;
    items: DataflowNodePaletteEntry[];
}[];
export declare function buildDataflowNodePaletteEntries(nodeDefinitions: readonly FlowNodeDefinition[]): DataflowNodePaletteEntry[];
export declare function buildDataflowNodePaletteEntry(definition: FlowNodeDefinition): DataflowNodePaletteEntry;
export {};

// core/dataflow-parameter-state.d.ts
/** Copyright (c) Scott A Dixon */
import type { FlowNodeDefinition as WireFlowNodeDefinition, FlowParameterDefinition as WireFlowParameterDefinition } from "@battersea/flow";
import type { FlowPortSide } from "./flow-node-ports.js";
import type { FlowStudioWorkspaceState } from "./flow-persistence.js";
export declare function updateWorkspaceNodeParameterValue(options: {
    definition: WireFlowNodeDefinition;
    nodeId: string;
    parameter: WireFlowParameterDefinition;
    remove?: boolean;
    value: unknown;
    workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState;
export declare function updateWorkspaceNodePortParameterValue(options: {
    definition: WireFlowNodeDefinition;
    nodeId: string;
    parameter: WireFlowParameterDefinition;
    portId: string;
    remove?: boolean;
    side: Extract<FlowPortSide, "input" | "output">;
    value: unknown;
    workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState;

// core/dataflow-undo.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Implements dataflow-specific structural snapshotting and restore logic for
 * authoring undo. Backed directly by `react-amnesia`.
 */
import type { Amnesia } from "react-amnesia";
import type { FlowStudioEdge, FlowStudioNode } from "./dataflow-editor-state.js";
import type { FlowStudioWorkspaceState } from "./flow-persistence.js";
export interface DataflowStructuralSnapshot {
    edges: FlowStudioEdge[];
    nodes: FlowStudioNode[];
}
export interface DataflowUndoHistoryState {
    restoreLayout: boolean;
    snapshot: DataflowStructuralSnapshot;
}
export declare function buildDataflowUndoHistoryState(snapshot: DataflowStructuralSnapshot, restoreLayout: boolean): DataflowUndoHistoryState;
export declare function buildDataflowStructuralSnapshot(workspace: Pick<FlowStudioWorkspaceState, "edges" | "nodes">): DataflowStructuralSnapshot;
export declare function areDataflowStructuralSnapshotsEqual(left: DataflowStructuralSnapshot, right: DataflowStructuralSnapshot): boolean;
export declare function areDataflowStructuralSnapshotsExactlyEqual(left: DataflowStructuralSnapshot, right: DataflowStructuralSnapshot): boolean;
/**
 * Push a structural-or-layout transformation onto the supplied amnesia store
 * and return the resulting workspace.
 *
 * `previousState` is the last `(snapshot, restoreLayout)` pair amnesia
 * captured (initial state, or the most recent push / amend / undo / redo
 * target). It is *not* necessarily derived from `workspace`: when the
 * workspace was live-mutated outside the undo path (e.g. node dragging via
 * `setWorkspace` direct), the live mutation is silently absorbed into the
 * next commit, matching the in-house `SnapshotHistory.getPresent()`
 * semantics.
 *
 * For layout commits (`restoreLayout: true`) the undo closure restores the
 * snapshot of the workspace at commit time *with* its layout, so undoing a
 * layout edit always snaps back to the layout the user had immediately
 * before the edit.
 *
 * For structural commits (`restoreLayout: false`) the undo closure restores
 * the previous lastCaptured state — which may itself carry
 * `restoreLayout: true` from a prior layout commit. Live layout drift
 * between two structural commits is preserved (the next structural commit's
 * undo restores the user's current layout).
 */
export declare function commitDataflowSnapshotChange(input: {
    amnesia: Amnesia;
    compareSnapshots: (left: DataflowStructuralSnapshot, right: DataflowStructuralSnapshot) => boolean;
    label: string;
    options?: {
        mergeKey?: string | null;
    };
    previousState: DataflowUndoHistoryState;
    restoreLayout: boolean;
    restoreState: (state: DataflowUndoHistoryState) => void;
    transform: (workspace: FlowStudioWorkspaceState) => FlowStudioWorkspaceState;
    workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState;
/**
 * Retroactively record a layout transition onto the amnesia stack.
 *
 * `previousSnapshot` is the layout the user had at the start of the gesture
 * (captured via `captureSnapshot` before live-dragging began). The current
 * workspace already reflects the post-gesture layout. This call inserts a
 * single undoable entry whose undo restores `previousSnapshot` with layout
 * and whose redo restores the current snapshot.
 */
export declare function pushDataflowPresentSnapshotChange(input: {
    amnesia: Amnesia;
    label: string;
    options?: {
        mergeKey?: string | null;
    };
    previousSnapshot: DataflowStructuralSnapshot;
    restoreLayout?: boolean;
    restoreState: (state: DataflowUndoHistoryState) => void;
    workspace: FlowStudioWorkspaceState;
}): boolean;
export declare function restoreDataflowStructuralWorkspace(options: {
    currentWorkspace: FlowStudioWorkspaceState;
    preserveLayout?: boolean;
    snapshot: DataflowStructuralSnapshot;
}): FlowStudioWorkspaceState;

// core/dataflow-workspace-selectors.d.ts
import type { FlowNodeDefinition } from "@battersea/flow";
import type { FlowStudioWorkspaceState } from "./flow-persistence.js";
import type { FlowStudioNode } from "./dataflow-editor-state.js";
import { type FlowStudioResolvedPort } from "./flow-node-ports.js";
export declare function findSelectedFlowStudioNode(workspace: FlowStudioWorkspaceState): FlowStudioNode | null;
export declare function buildRenderedFlowStudioNodes(workspace: FlowStudioWorkspaceState): FlowStudioNode[];
export declare function findSelectedFlowStudioPort(selectedNode: FlowStudioNode | null, selectedTarget: FlowStudioWorkspaceState["selectedTarget"]): FlowStudioResolvedPort | null;
export declare function resolveSelectedNodeDefinition(selectedNode: FlowStudioNode | null, nodeDefinitionLookup: Record<string, FlowNodeDefinition>): FlowNodeDefinition | null;

// core/flow-canvas-edges.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Resolves edge presentation for the editor's dataflow canvas.
 */
import type { FlowStudioEdge } from "./dataflow-editor-state.js";
import type { FlowStudioEdgeActivationPhase } from "./flow-edge-activation.js";
import type { FlowStudioNode } from "./dataflow-editor-state.js";
/**
 * Returns true when a token type carries an array/list of values rather than
 * a single value. Array-typed edges are rendered with a doubled stroke so
 * authors can tell at a glance that the wire moves a sequence.
 *
 * Token-type names follow two naming conventions in the manifest: a
 * `camelCaseArray` suffix (e.g. `prompt.fragmentArray`) and a
 * snake_case `_list` / `_ids` suffix (e.g. `media.asset_list`,
 * `story.character_ids`). Both indicate cardinality > 1.
 */
export declare function isArrayTokenType(tokenType: string | null | undefined): boolean;
export declare function resolveFlowCanvasEdge(edge: FlowStudioEdge, nodes: readonly FlowStudioNode[], activationPhase?: FlowStudioEdgeActivationPhase | null): FlowStudioEdge;

// core/flow-canvas-node.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Implements flow canvas node for the editor's dataflow workspace.
 */
import React from "react";
import { type Node, type NodeProps } from "@xyflow/react";
import { type FlowControllerPortPlacement } from "./flow-controller-port-placement.js";
import type { FlowStudioPortSelectionTarget } from "./dataflow-editor-state.js";
import type { FlowStudioNodeData } from "./flow-drag.js";
import { type FlowPortMoveDirection } from "./flow-port-move-controls.js";
import { buildFlowPortSlots, type FlowPortSide, type FlowStudioResolvedPort } from "./flow-node-ports.js";
export interface FlowStudioCanvasNodeData extends FlowStudioNodeData {
    canvasMovedPortPulse?: {
        nodeId: string;
        portId: string;
        replay: "a" | "b";
        side: FlowPortSide;
    } | null;
    canvasOnMovePort?: (target: {
        direction: FlowPortMoveDirection;
        nodeId: string;
        portId: string;
        side: FlowPortSide;
    }) => void;
    canvasOnPortSelect?: (target: {
        nodeId: string;
        portId: string;
        side: FlowStudioPortSelectionTarget["side"];
    }) => void;
    canvasSelectedPort?: FlowStudioPortSelectionTarget | null;
}
type FlowStudioCanvasNodeType = Node<FlowStudioCanvasNodeData, "flowStudio">;
export interface FlowStudioCanvasNodeViewModel {
    actionPorts: FlowStudioResolvedPort[];
    actionPortSlots: ReturnType<typeof buildFlowPortSlots>["slots"];
    automationPorts: FlowStudioResolvedPort[];
    automationPortSlots: ReturnType<typeof buildFlowPortSlots>["slots"];
    controllerPortPlacement: FlowControllerPortPlacement;
    canonicalDefinitionName: string;
    definitionName: string;
    hasController: boolean;
    inputPorts: FlowStudioResolvedPort[];
    inputPortSlots: ReturnType<typeof buildFlowPortSlots>["slots"];
    instanceName: string;
    layout: {
        handleGapPx: number;
        nodeHeightPx: number;
        nodeWidthPx: number;
        pillMaxWidthPx: number;
    };
    nodeClass: FlowStudioNodeData["nodeClass"];
    outputPorts: FlowStudioResolvedPort[];
    outputPortSlots: ReturnType<typeof buildFlowPortSlots>["slots"];
    reversed: boolean;
    signalPorts: FlowStudioResolvedPort[];
    signalPortSlots: ReturnType<typeof buildFlowPortSlots>["slots"];
}
export declare function resolveFlowStudioCanvasNodeViewModel(data: Partial<FlowStudioNodeData> | null | undefined): FlowStudioCanvasNodeViewModel;
export declare function FlowStudioCanvasNode({ data, id, selected, }: NodeProps<FlowStudioCanvasNodeType>): React.JSX.Element;
export declare function handlePortSelect(event: React.MouseEvent | React.PointerEvent, onPortSelect: ((target: {
    nodeId: string;
    portId: string;
    side: FlowStudioPortSelectionTarget["side"];
}) => void) | undefined, nodeId: string, side: FlowStudioPortSelectionTarget["side"], port: FlowStudioResolvedPort | undefined, focusTargetId?: string): void;
export declare function resolveFlowPortTooltipText(port: FlowStudioResolvedPort | undefined, fallbackLabel: string): string;
export {};

// core/flow-control-layout.d.ts
import type { FlowParameterDefinition as WireFlowParameterDefinition } from "@battersea/flow";
export interface FlowControlLayoutGroup {
    parameters: WireFlowParameterDefinition[];
    layout: "row" | "stack";
}
export declare function groupControlsForDetailPane(parameters: WireFlowParameterDefinition[]): FlowControlLayoutGroup[];

// core/flow-controller-port-placement.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Tracks editor-local placement of controller ports on a node's controller axis.
 */
import type { FlowNodeClass as WireFlowNodeClass } from "@battersea/flow";
export type FlowControllerPortPlacement = "default" | "swapped";
export type FlowControllerPortVisualSide = "bottom" | "left" | "right" | "top";
export declare function normalizeFlowControllerPortPlacement(value: unknown): FlowControllerPortPlacement;
export declare function toggleFlowControllerPortPlacement(placement: FlowControllerPortPlacement | null | undefined): FlowControllerPortPlacement;
export declare function hasRotatedControllerPortAxis(nodeClass: WireFlowNodeClass | null | undefined): boolean;
export declare function resolveFlowControllerPortVisualSide(options: {
    nodeClass?: WireFlowNodeClass | null;
    placement: FlowControllerPortPlacement | null | undefined;
    side: "action" | "automation" | "signal";
}): FlowControllerPortVisualSide;

// core/flow-drag.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Implements flow drag for the editor's dataflow workspace.
 */
import type { XYPosition, Node } from "@xyflow/react";
import type { FlowNode as WireFlowNode, FlowNodeDefinition as WireFlowNodeDefinition } from "@battersea/flow";
import type { FlowControllerPortPlacement } from "./flow-controller-port-placement.js";
import type { FlowStudioPortLike } from "./flow-node-ports.js";
import type { FlowStudioPortOrder } from "./flow-port-order.js";
export interface FlowStudioNodeData extends Record<string, unknown> {
    actionPorts: FlowStudioPortLike[];
    automationPorts: FlowStudioPortLike[];
    controllerPortPlacement?: FlowControllerPortPlacement;
    definitionName: string;
    hasController?: boolean;
    inputPorts: FlowStudioPortLike[];
    instanceName: string;
    longDescription: string;
    nodeClass: WireFlowNodeDefinition["kind"];
    parameterValues: Record<string, unknown>;
    portParameterValues?: WireFlowNode["port_parameter_values"];
    portOrder?: FlowStudioPortOrder;
    outputPorts: FlowStudioPortLike[];
    portNames?: Record<string, Record<string, string> | undefined>;
    signalPorts: FlowStudioPortLike[];
    shortDescription: string;
}
export interface FlowNodeDragPayload {
    actionPorts: FlowStudioPortLike[];
    automationPorts: FlowStudioPortLike[];
    definitionName: string;
    dragOffset?: XYPosition;
    hasController: boolean;
    inputPorts: FlowStudioPortLike[];
    longDescription: string;
    nodeClass: WireFlowNodeDefinition["kind"];
    outputPorts: FlowStudioPortLike[];
    parameterValues: Record<string, unknown>;
    signalPorts: FlowStudioPortLike[];
    shortDescription: string;
    title: string;
}
export declare const FLOW_NODE_PALETTE_DRAG_ID_PREFIX = "dataflow-palette:";
export declare function buildFlowNodePaletteDragId(className: string): string;
export declare function isFlowNodePaletteDragId(value: string | number): boolean;
export declare function createFlowNodeDragPayload(nodeDefinition: WireFlowNodeDefinition, dragOffset?: XYPosition): FlowNodeDragPayload;
export declare function serialiseFlowNodeDragPayload(payload: FlowNodeDragPayload): string;
export declare function parseFlowNodeDragPayload(rawPayload: string): FlowNodeDragPayload | null;
export declare function createDroppedFlowNode(params: {
    nextIndex: number;
    payload: FlowNodeDragPayload;
    position: XYPosition;
}): Node<FlowStudioNodeData>;
export declare function resolveDroppedFlowNodePosition(params: {
    cursorPosition: XYPosition;
    payload: FlowNodeDragPayload;
}): XYPosition;
export declare const DATAFLOW_CANVAS_DROPZONE_ID = "dataflow-canvas-dropzone";

// core/flow-dynamic-port-template.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Shared helpers for dynamic-port name templates ("input-{index}") and for
 * computing the old→new id rename map used when the editor renumbers a
 * dynamic-port group. Both port delete and port reorder rely on this so the
 * saved flow's port ids encode the user's intended order (the engine sorts
 * dynamic input ports by numeric suffix, so visual order has to be expressed
 * in the id itself, not in a parallel `port_order` field the runtime ignores).
 */
import type { FlowDynamicPortGroup as WireFlowDynamicPortGroup, FlowNodeDefinition as WireFlowNodeDefinition } from "@battersea/flow";
import type { FlowPortSide } from "./flow-node-ports.js";
/**
 * Parses a port id against a `name_template` like `"input-{index}"`. Returns
 * the numeric index when the id matches the template, or null when it doesn't
 * (e.g. the id is a fixed-port name like `"prompt"`).
 */
export declare function dynamicPortTemplateIndex(template: string, portId: string): number | null;
/**
 * Renders a port id from a template and an index, e.g. `("input-{index}", 2)`
 * → `"input-2"`.
 */
export declare function dynamicPortTemplateId(template: string, index: number): string;
/**
 * Returns the dynamic port groups defined for the given side. Action,
 * signal and automation sides have no dynamic groups in the current
 * schema; only `input` and `output` carry variadic groups.
 */
export declare function dynamicPortGroupsForSide(definition: WireFlowNodeDefinition, side: FlowPortSide): readonly WireFlowDynamicPortGroup[];
/**
 * Builds a `oldPortId -> newPortId` rename map for a side, given the
 * desired visual order of ports (by their current ids). Fixed ports keep
 * their ids; each dynamic group's surviving members are renumbered into a
 * contiguous id space (`0..N-1`) in the order they appear in the visual
 * sequence.
 *
 * Passing the same id list for both the visual order and the existing
 * order is a no-op (identity map). The caller is responsible for then
 * applying the map to `port_names`, `port_parameter_values`, and any edges
 * whose handles reference renamed ids.
 */
export declare function buildDynamicPortRenameMap(options: {
    definition: WireFlowNodeDefinition;
    side: FlowPortSide;
    visualOrderOldIds: readonly string[];
}): Map<string, string>;

// core/flow-edge-activation.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Resolves live edge-activation state for the editor's dataflow workspace.
 */
export interface FlowActivityRecord {
    phase: string;
    detail?: unknown;
}
export declare const FLOW_STUDIO_EDGE_ACTIVE_FALLBACK_MS = 800;
export declare const FLOW_STUDIO_EDGE_FADE_OUT_MS = 1200;
export declare const FLOW_STUDIO_EDGE_MIN_ACTIVE_MS = 280;
export type FlowStudioEdgeActivationPhase = "active" | "fading";
export type FlowStudioEdgeActivationState = Record<string, FlowStudioEdgeActivationPhase>;
export interface FlowStudioEdgeActivityEvent {
    edgeIds: string[];
    transition: "activate" | "fade";
}
export declare function resolveFlowStudioEdgeFadeDelayMs(options: {
    activatedAtMs: number;
    minimumActiveMs?: number;
    nowMs: number;
}): number;
export declare function resolveFlowStudioEdgeActivityEvent<T extends FlowActivityRecord>(record: T): FlowStudioEdgeActivityEvent | null;
export declare function pruneFlowStudioEdgeActivationState(state: FlowStudioEdgeActivationState, validEdgeIds: ReadonlySet<string>): FlowStudioEdgeActivationState;

// core/flow-edge-bridges.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Builds and mutates manual edge bridge geometry for flow-studio edges.
 */
import type { AuthoringGraphBridgeGapInterval, AuthoringGraphBridgeLocation, AuthoringGraphInsertionInterval, ResolvedAuthoringGraphBridgeGeometry } from "../graph.js";
import { updateBridgeIndicesForWaypointInsertion, updateBridgeIndicesForWaypointRemoval } from "../graph.js";
import type { FlowStudioEdge, FlowStudioEdgeBridge, FlowStudioPoint } from "./dataflow-editor-state.js";
import type { FlowEdgeWaypointSegment } from "./flow-edge-waypoints.js";
export type FlowEdgeBridgeLocation = AuthoringGraphBridgeLocation;
export type FlowEdgeInsertionInterval = AuthoringGraphInsertionInterval;
export type FlowEdgeBridgeGapInterval = AuthoringGraphBridgeGapInterval;
export type ResolvedFlowEdgeBridgeGeometry = ResolvedAuthoringGraphBridgeGeometry;
export { updateBridgeIndicesForWaypointInsertion, updateBridgeIndicesForWaypointRemoval, };
export declare function createBridgeForSegment(segmentIndex: number): FlowStudioEdgeBridge;
export declare function createBridgeForInterval(interval: FlowEdgeInsertionInterval): FlowStudioEdgeBridge;
export declare function insertBridgeIntoEdge(edge: FlowStudioEdge, bridge: FlowStudioEdgeBridge): FlowStudioEdge;
export declare function moveBridgeInEdge(edge: FlowStudioEdge, bridgeIndex: number, location: FlowEdgeBridgeLocation): FlowStudioEdge;
export declare function resizeBridgeInEdge(edge: FlowStudioEdge, bridgeIndex: number, gap: number): FlowStudioEdge;
export declare function removeBridgeFromEdge(edge: FlowStudioEdge, bridgeIndex: number): FlowStudioEdge;
export declare function resolveNearestBridgeLocation(segments: readonly FlowEdgeWaypointSegment[], point: FlowStudioPoint): FlowEdgeBridgeLocation | null;
export declare function resolveBridgeGeometry(segments: readonly FlowEdgeWaypointSegment[], bridge: FlowStudioEdgeBridge): ResolvedFlowEdgeBridgeGeometry | null;
export declare function buildVisibleEdgePathWithBridges(segments: readonly FlowEdgeWaypointSegment[], bridges: readonly FlowStudioEdgeBridge[]): string;
export declare function resolveBridgeGapIntervals(segments: readonly FlowEdgeWaypointSegment[], bridges: readonly FlowStudioEdgeBridge[]): FlowEdgeBridgeGapInterval[];
export declare function buildEdgeInsertionIntervals(segments: readonly FlowEdgeWaypointSegment[], bridges: readonly FlowStudioEdgeBridge[]): FlowEdgeInsertionInterval[];
export declare function resolveInsertionIntervalMidpoint(segments: readonly FlowEdgeWaypointSegment[], interval: FlowEdgeInsertionInterval): FlowStudioPoint | null;
export declare function resolveNearestInsertionInterval(segments: readonly FlowEdgeWaypointSegment[], bridges: readonly FlowStudioEdgeBridge[], point: FlowStudioPoint): FlowEdgeInsertionInterval | null;
export declare function resolveBridgeGapFromHandle(geometry: ResolvedFlowEdgeBridgeGeometry, handlePosition: FlowStudioPoint): number;

// core/flow-edge-waypoints.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Builds and mutates editable bezier waypoint geometry for flow-studio edges.
 */
import type { BuildAuthoringGraphWaypointSegmentsOptions, AuthoringGraphWaypointSegment } from "../graph.js";
import type { FlowStudioEdge, FlowStudioEdgeWaypoint, FlowStudioPoint } from "./dataflow-editor-state.js";
export type FlowEdgeWaypointSegment = AuthoringGraphWaypointSegment;
export type BuildFlowEdgeWaypointSegmentsOptions = BuildAuthoringGraphWaypointSegmentsOptions;
export declare function buildFlowEdgeWaypointSegments(options: BuildFlowEdgeWaypointSegmentsOptions): FlowEdgeWaypointSegment[];
export declare function buildFlowEdgeWaypointPath(options: BuildFlowEdgeWaypointSegmentsOptions): string;
export declare function evaluateCubicBezierPoint(segment: FlowEdgeWaypointSegment, t: number): FlowStudioPoint;
export declare function resolveNearestWaypointSegmentIndex(segments: readonly FlowEdgeWaypointSegment[], point: FlowStudioPoint): number | null;
export declare function createWaypointForSegment(segment: FlowEdgeWaypointSegment): FlowStudioEdgeWaypoint;
export declare function createWaypointForSegmentAt(segment: FlowEdgeWaypointSegment, t: number): FlowStudioEdgeWaypoint;
export declare function insertWaypointIntoEdge(edge: FlowStudioEdge, segmentIndex: number, waypoint: FlowStudioEdgeWaypoint, segmentT?: number): FlowStudioEdge;
export declare function moveWaypointInEdge(edge: FlowStudioEdge, waypointIndex: number, position: FlowStudioPoint): FlowStudioEdge;
export declare function moveWaypointHandleInEdge(edge: FlowStudioEdge, waypointIndex: number, handleKind: "inHandle" | "outHandle", independent: boolean, position: FlowStudioPoint): FlowStudioEdge;
export declare function removeWaypointFromEdge(edge: FlowStudioEdge, waypointIndex: number): FlowStudioEdge;

// core/flow-enum-switch.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Implements flow enum switch for the editor's dataflow workspace.
 */
export interface FlowEnumOption {
    label: string;
    value: string;
}
export declare function flowEnumUsesSwitch(options: FlowEnumOption[]): boolean;

// core/flow-layout.d.ts
/** Copyright (c) Scott A Dixon */
import type { LayoutGraph, LayoutPositions } from "../graph/layout/types.js";
import type { FlowStudioWorkspaceState } from "./flow-persistence.js";
/** Preserves authored edge direction and every node, including disconnected nodes. */
export declare function buildDataflowLayoutGraph(workspace: FlowStudioWorkspaceState): LayoutGraph;
/** Layout changes only positions; parameter values, port order and authored edge paths survive. */
export declare function applyDataflowLayout(workspace: FlowStudioWorkspaceState, positions: LayoutPositions): FlowStudioWorkspaceState;

// core/flow-list-control.d.ts
import type { FlowParameterDefinition } from "@battersea/flow";
export declare function flowListUsesSingleSelect(parameter: FlowParameterDefinition): boolean;
export declare function resolveSingleSelectValue(value: unknown): string;
export declare function normaliseSingleSelectValue(value: string): string[];

// core/flow-multi-select.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Implements flow multi select for the editor's dataflow workspace.
 */
export interface FlowMultiSelectOption {
    label: string;
    value: string;
}
export declare function selectAllFlowMultiSelectValues(options: FlowMultiSelectOption[]): string[];
export declare function selectNoFlowMultiSelectValues(): string[];
export declare function flowMultiSelectHasAllValues(options: FlowMultiSelectOption[], selectedValues: string[]): boolean;

// core/flow-node-definitions.d.ts
import type { FlowNodeClass as WireFlowNodeClass, FlowNode as WireFlowNode, FlowNodeDefinition as WireFlowNodeDefinition, FlowParameterDefinition as WireFlowParameterDefinition, FlowPort as WireFlowPort } from "@battersea/flow";
import { type FlowStudioPortLike, type FlowStudioResolvedPort } from "./flow-node-ports.js";
import { type FlowControllerPortPlacement } from "./flow-controller-port-placement.js";
import { type FlowStudioPortOrder } from "./flow-port-order.js";
export type FlowParameterValues = Record<string, unknown>;
export type FlowPortParameterValues = WireFlowNode["port_parameter_values"];
export interface FlowStudioResolvedNodeData extends Record<string, unknown> {
    actionPorts: FlowStudioPortLike[];
    automationPorts: FlowStudioPortLike[];
    controllerPortPlacement?: FlowControllerPortPlacement;
    definitionName: string;
    hasController?: boolean;
    inputPorts: FlowStudioPortLike[];
    instanceName: string;
    longDescription: string;
    nodeClass: WireFlowNodeClass;
    parameterValues: FlowParameterValues;
    portParameterValues?: FlowPortParameterValues;
    portOrder?: FlowStudioPortOrder;
    outputPorts: FlowStudioPortLike[];
    portNames?: WireFlowNode["port_names"];
    signalPorts: FlowStudioPortLike[];
    shortDescription: string;
}
export declare function createFlowDefinitionLookup(definitions: WireFlowNodeDefinition[]): Record<string, WireFlowNodeDefinition>;
export declare function parameterHasController(parameter: WireFlowParameterDefinition): boolean;
export declare function definitionHasControllers(definition: WireFlowNodeDefinition): boolean;
export declare function formatFlowDefinitionTitle(className: string): string;
export declare function buildDefaultFlowNodeId(className: string, nextIndex: number): string;
export declare function buildDefaultInstanceName(className: string, nextIndex: number): string;
export declare function nodeDefinitionIsActivatable(definition: WireFlowNodeDefinition): boolean;
export declare function getActivationParameterNames(definition: WireFlowNodeDefinition): string[];
export declare function getPersistedParameters(definition: WireFlowNodeDefinition): WireFlowParameterDefinition[];
export declare function getActivationParameters(definition: WireFlowNodeDefinition): WireFlowParameterDefinition[];
export declare function getEffectiveParameterValue(definition: WireFlowNodeDefinition, parameterName: string, parameterValues: FlowParameterValues): unknown;
export declare function getEffectivePortParameterValue(options: {
    definition: WireFlowNodeDefinition;
    parameter: WireFlowParameterDefinition;
    parameterValues?: FlowPortParameterValues | null;
    portId: string;
    side: "input" | "output";
}): unknown;
export declare function buildDefaultParameterValues(definition: WireFlowNodeDefinition): FlowParameterValues;
export declare function expandDefinitionPorts(definition: WireFlowNodeDefinition, parameterValues: FlowParameterValues, side: "input" | "output"): WireFlowPort[];
export declare function resolveActionPorts(definition: WireFlowNodeDefinition, parameterValues: FlowParameterValues, portNames?: WireFlowNode["port_names"] | null): FlowStudioResolvedPort[];
export declare function resolveSignalPorts(definition: WireFlowNodeDefinition, parameterValues: FlowParameterValues, portNames?: WireFlowNode["port_names"] | null): FlowStudioResolvedPort[];
export declare function resolveAutomationPorts(definition: WireFlowNodeDefinition, portNames?: WireFlowNode["port_names"] | null): FlowStudioResolvedPort[];
export declare function resolveInputPorts(definition: WireFlowNodeDefinition, parameterValues: FlowParameterValues, portNames?: WireFlowNode["port_names"] | null): FlowStudioResolvedPort[];
export declare function resolveOutputPorts(definition: WireFlowNodeDefinition, parameterValues: FlowParameterValues, portNames?: WireFlowNode["port_names"] | null): FlowStudioResolvedPort[];
export declare function prunePortNamesForResolvedPorts(options: {
    actionPorts: readonly FlowStudioResolvedPort[];
    automationPorts: readonly FlowStudioResolvedPort[];
    inputPorts: readonly FlowStudioResolvedPort[];
    outputPorts: readonly FlowStudioResolvedPort[];
    portNames?: WireFlowNode["port_names"] | null;
    signalPorts: readonly FlowStudioResolvedPort[];
}): WireFlowNode["port_names"] | undefined;
export declare function formatFlowPortLabel(portName: string): string;
export declare function formatFlowParameterLabel(parameterName: string): string;
export declare function resolveParameterEditorValue(definition: WireFlowNodeDefinition, parameter: WireFlowParameterDefinition, parameterValues: FlowParameterValues): unknown;
export declare function isStringListParameter(parameter: WireFlowParameterDefinition): boolean;
export declare function isSupportedFlowParameter(parameter: WireFlowParameterDefinition): boolean;
export declare function getParameterOptions(parameter: WireFlowParameterDefinition): string[];
export declare function getNodeClassLabel(nodeClass: WireFlowNodeClass): string;
export declare function buildResolvedNodeData(options: {
    controllerPortPlacement?: FlowControllerPortPlacement | null;
    definition: WireFlowNodeDefinition;
    instanceName: string;
    portOrder?: WireFlowNode["port_order"] | null;
    parameterValues: FlowParameterValues;
    portParameterValues?: FlowPortParameterValues | null;
    portNames?: WireFlowNode["port_names"] | null;
}): FlowStudioResolvedNodeData;
export declare function cloneFlowPortParameterValues(portParameterValues: FlowPortParameterValues | null | undefined): FlowPortParameterValues | undefined;

// core/flow-node-drop-feedback.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Implements flow node drop feedback for the editor's dataflow workspace.
 */
export type FlowNodeDropFailureReason = "canvas-unavailable" | "invalid-payload" | "node-build-failed";
export interface FlowNodeDropNotification {
    message: string;
    title: string;
}
export declare function buildFlowNodeDropFailureNotification(params: {
    nodeTitle: string;
    reason: FlowNodeDropFailureReason;
}): FlowNodeDropNotification;

// core/flow-node-ports.d.ts
import type { Edge } from "@xyflow/react";
import type { FlowNode as WireFlowNode, FlowPortMode, FlowPortPhase, FlowNodeClass as WireFlowNodeClass, FlowParameterDefinition as WireFlowParameterDefinition } from "@battersea/flow";
import { type AuthoringGraphHandleDescriptor } from "../graph.js";
import { type FlowControllerPortPlacement } from "./flow-controller-port-placement.js";
import type { FlowStudioEdgeData } from "./dataflow-editor-state.js";
export type FlowPortSide = "action" | "automation" | "input" | "output" | "signal";
export type FlowConnectionKind = "signal" | "token";
export type FlowStudioPortNames = WireFlowNode["port_names"];
export type FlowStudioPortDisplayClass = "inline" | "sink" | "source";
export interface FlowStudioResolvedPort {
    mode?: FlowPortMode;
    phase?: FlowPortPhase;
    acceptedTokenTypes?: string[];
    displayClass: FlowStudioPortDisplayClass;
    id: string;
    label: string;
    longDescription?: string;
    name?: string;
    parameters?: WireFlowParameterDefinition[];
    shortDescription?: string;
    side: FlowPortSide;
    tokenType?: string;
}
export type FlowStudioPortLike = FlowStudioResolvedPort | string;
export interface FlowPortConnectionLike {
    source?: string | null;
    sourceHandle?: string | null;
    target?: string | null;
    targetHandle?: string | null;
}
interface FlowConnectionDescriptorNodeLike extends FlowStudioConnectionNodeLike {
    data?: FlowStudioConnectionNodeLike["data"] & {
        controllerPortPlacement?: FlowControllerPortPlacement;
        nodeClass?: WireFlowNodeClass | null;
    };
}
export interface FlowPortSlot {
    handleId: string;
    index: number;
    offsetPixels: number;
    offsetPercent: number;
    sideCount: number;
}
export interface FlowPortSlotLayout {
    handleGapPx: number;
    nodeHeightPx: number;
    nodeWidthPx: number;
    pillMaxWidthPx: number;
    slots: FlowPortSlot[];
}
export interface FlowPortSideCounts {
    actionCount: number;
    automationCount: number;
    inputCount: number;
    outputCount: number;
    signalCount: number;
}
type FlowStudioConnectionNodeLike = {
    data?: {
        actionPorts?: readonly FlowStudioPortLike[];
        automationPorts?: readonly FlowStudioPortLike[];
        inputPorts?: readonly FlowStudioPortLike[];
        outputPorts?: readonly FlowStudioPortLike[];
        signalPorts?: readonly FlowStudioPortLike[];
    };
    id: string;
};
export declare function getFlowPortLabel(portId: string): string;
export declare function resolveFlowPortAliasLabel(portId: string, alias?: string | null): string;
export declare function normaliseFlowPortAlias(alias: string): string | null;
export declare function buildResolvedFlowPort(options: {
    mode?: FlowPortMode;
    phase?: FlowPortPhase;
    acceptedTokenTypes?: string[] | null;
    displayClass?: FlowStudioPortDisplayClass | null;
    id: string;
    longDescription?: string | null;
    name?: string | null;
    nodeClass?: WireFlowNodeClass | null;
    parameters?: readonly WireFlowParameterDefinition[] | null;
    shortDescription?: string | null;
    side: FlowPortSide;
    tokenType?: string | null;
}): FlowStudioResolvedPort;
export declare function resolveFlowStudioResolvedPorts(ports: readonly FlowStudioPortLike[], side: FlowPortSide, nodeClass?: WireFlowNodeClass | null): FlowStudioResolvedPort[];
export declare function resolveFlowPortDisplayClass(nodeClass?: WireFlowNodeClass | null, explicitDisplayClass?: FlowStudioPortDisplayClass | null): FlowStudioPortDisplayClass;
export declare function cloneFlowStudioPortNames(portNames: FlowStudioPortNames | null | undefined): FlowStudioPortNames | undefined;
export declare function getFlowPortAlias(portNames: FlowStudioPortNames | null | undefined, side: FlowPortSide, portId: string): string | undefined;
export declare function setFlowPortAlias(options: {
    nextAlias: string | null;
    portId: string;
    portNames: FlowStudioPortNames | null | undefined;
    side: FlowPortSide;
}): FlowStudioPortNames | undefined;
export declare function pruneFlowPortAliases(portNames: FlowStudioPortNames | null | undefined, side: FlowPortSide, validPortIds: readonly string[]): FlowStudioPortNames | undefined;
export declare function hasFlowStudioPortNames(portNames: FlowStudioPortNames | null | undefined): boolean;
export declare function createFlowPortHandleId(side: FlowPortSide, index: number): string;
export declare function buildFlowHandleDescriptors(options: {
    actionPorts?: readonly FlowStudioPortLike[];
    automationPorts?: readonly FlowStudioPortLike[];
    controllerPortPlacement?: FlowControllerPortPlacement;
    inputPorts?: readonly FlowStudioPortLike[];
    nodeClass?: WireFlowNodeClass | null;
    outputPorts?: readonly FlowStudioPortLike[];
    signalPorts?: readonly FlowStudioPortLike[];
}): AuthoringGraphHandleDescriptor<FlowConnectionKind>[];
export declare function resolveFlowConnectionKind(connection: FlowPortConnectionLike): FlowConnectionKind | null;
export declare function buildFlowPortSlots(side: FlowPortSide, sideCounts: FlowPortSideCounts, nodeClass?: WireFlowNodeClass | null): FlowPortSlotLayout;
export declare function pruneEdgesForNodeCardinality(edges: Edge<FlowStudioEdgeData>[], nodeId: string, side: FlowPortSide, nextCount: number): Edge<FlowStudioEdgeData>[];
export declare function canAttachConnectionToPorts(edges: Array<Pick<Edge<FlowStudioEdgeData>, "data" | "source" | "sourceHandle" | "target" | "targetHandle">>, connection: FlowPortConnectionLike, nodes?: ReadonlyArray<FlowConnectionDescriptorNodeLike>): boolean;
export declare function connectionConflictsWithExistingEdge(edges: Array<Pick<Edge<FlowStudioEdgeData>, "data" | "source" | "sourceHandle" | "target" | "targetHandle">>, connection: FlowPortConnectionLike): boolean;
export declare function resolveConnectionFromHandlePair(params: {
    fromHandle: {
        id?: string | null;
        nodeId: string;
    } | null;
    toHandle: {
        id?: string | null;
        nodeId: string;
    } | null;
}): FlowPortConnectionLike | null;
export declare function connectionUsesCompatibleTokenTypes(options: {
    connection: FlowPortConnectionLike;
    nodes: ReadonlyArray<FlowConnectionDescriptorNodeLike>;
}): boolean;
export declare function resolveConnectionSourceTokenType(options: {
    connection: FlowPortConnectionLike;
    nodes: ReadonlyArray<FlowConnectionDescriptorNodeLike>;
}): string | null;
export {};

// core/flow-persistence.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Persists and restores flow persistence for the editor's dataflow workspace.
 */
import type { Node } from "@xyflow/react";
import type { FlowDocument as WireFlowDocument, FlowNodeDefinition as WireFlowNodeDefinition } from "@battersea/flow";
import type { FlowStudioEdge, FlowStudioSelectionTarget } from "./dataflow-editor-state.js";
import type { FlowStudioNodeData } from "./flow-drag.js";
export interface FlowStudioWorkspaceState {
    baselineFlow: WireFlowDocument | null;
    description: string;
    draftFlowKey: string;
    edges: FlowStudioEdge[];
    nodes: Array<Node<FlowStudioNodeData>>;
    /**
     * Flow-wide rendering encoding. Nodes with their per-node
     * `output_encoding` set to `"inherit"` (the default) read from this
     * field. Mirrors {@link WireFlowDocument.output_encoding}. One of
     * `"markdown"`, `"xml"`, or `"plain"`.
     */
    outputEncoding: string;
    /**
     * Flow-wide field delimiter used when emitting the `plain` encoding via
     * inherit. Mirrors {@link WireFlowDocument.plain_fragment_delimiter}.
     * Catalog enum token: `comma`, `blank_line`, `newline`, `space`, `none`.
     */
    plainFragmentDelimiter: string;
    /**
     * Flow-wide whitespace handling. Nodes with their per-node
     * `whitespace_mode` set to `"inherit"` read from this field.
     * Mirrors {@link WireFlowDocument.whitespace_mode}.
     */
    whitespaceMode: string;
    selectedFlowKey: string;
    selectedTarget: FlowStudioSelectionTarget;
    title: string;
}
/**
 * Default flow-wide encoding when a workspace lacks one (legacy state from
 * before this field existed, or a freshly-created blank flow). Matches the
 * engine-side `default_flow_output_encoding` so old saves stay byte-for-byte.
 */
export declare const DEFAULT_FLOW_OUTPUT_ENCODING = "xml";
/**
 * Default flow-wide plain-fragment delimiter when a workspace lacks one.
 * Matches the engine-side `default_flow_plain_fragment_delimiter`.
 */
export declare const DEFAULT_FLOW_PLAIN_FRAGMENT_DELIMITER = "blank_line";
/**
 * Default flow-wide whitespace mode when a workspace lacks one. Matches the
 * engine-side `default_flow_whitespace_mode`.
 */
export declare const DEFAULT_FLOW_WHITESPACE_MODE = "trim";
/**
 * This editor's namespace within a flow document's client-scoped layout.
 *
 * Layout is a map of writer namespace to opaque blob; the engine reads the key
 * and nothing below it. Versioning the key is what lets this editor's canvas
 * dialect change without an engine release, and what keeps a second client
 * rendering the same flow from writing an incompatible shape to the same place.
 */
export declare const FLOW_EDITOR_LAYOUT_KEY = "flow_builder_v1";
export declare function buildDefaultFlowWorkspace(): FlowStudioWorkspaceState;
export declare function createBlankFlowDocument(): WireFlowDocument;
export declare function normalizeFlowWorkspaceState(value: unknown): FlowStudioWorkspaceState;
export declare function reconcileFlowWorkspaceDefinitions(workspace: FlowStudioWorkspaceState, definitionLookup: Record<string, WireFlowNodeDefinition>): FlowStudioWorkspaceState;
export declare function buildFlowDocumentFromWorkspace(params: {
    baselineFlow?: WireFlowDocument | null;
    description: string;
    draftFlowKey: string;
    edges: FlowStudioEdge[];
    nodes: Array<Node<FlowStudioNodeData>>;
    /** Optional override; defaults to {@link DEFAULT_FLOW_OUTPUT_ENCODING}. */
    outputEncoding?: string;
    /** Optional override; defaults to {@link DEFAULT_FLOW_PLAIN_FRAGMENT_DELIMITER}. */
    plainFragmentDelimiter?: string;
    /** Optional override; defaults to {@link DEFAULT_FLOW_WHITESPACE_MODE}. */
    whitespaceMode?: string;
    title: string;
}): WireFlowDocument;
export declare function buildFlowValidationDocument(params: {
    workspace: Pick<FlowStudioWorkspaceState, "description" | "draftFlowKey" | "edges" | "nodes" | "title"> & Partial<Pick<FlowStudioWorkspaceState, "baselineFlow" | "outputEncoding" | "plainFragmentDelimiter" | "whitespaceMode">>;
}): WireFlowDocument;
export declare function buildFlowSaveDocument(params: {
    titleOverride?: string;
    workspace: Pick<FlowStudioWorkspaceState, "description" | "draftFlowKey" | "edges" | "nodes" | "title"> & Partial<Pick<FlowStudioWorkspaceState, "baselineFlow" | "outputEncoding" | "plainFragmentDelimiter" | "whitespaceMode">>;
}): WireFlowDocument | null;
export declare function buildFlowWorkspaceFromDocument(params: {
    definitions: WireFlowNodeDefinition[];
    document: WireFlowDocument;
}): FlowStudioWorkspaceState;
export declare function areFlowDocumentsEqual(left: WireFlowDocument, right: WireFlowDocument): boolean;
export declare function slugifyFlowKey(title: string): string;
export declare function findNextFlowNodeIndex(nodes: Array<Node<FlowStudioNodeData>>): number;
export declare function removeNodeFromWorkspace(workspace: FlowStudioWorkspaceState, nodeId: string): FlowStudioWorkspaceState;
export declare function renameNodeInstanceInWorkspace(workspace: FlowStudioWorkspaceState, nodeId: string, instanceName: string): FlowStudioWorkspaceState;

// core/flow-port-delete.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Deletes a single variadic (dynamic-group) port from a flow node in one step.
 *
 * Dynamic ports are generated positionally from a count parameter
 * (`value_0 … value_{count-1}`), so shrinking the count only ever drops the
 * highest index. Deleting an arbitrary port therefore means: drop the target's
 * alias/parameters/edges, renumber the surviving dynamic ports down into the
 * contiguous id space the new count produces, shift their aliases, per-port
 * parameters, and edges to follow, and decrement the count parameter. This
 * replaces the manual "reorder-to-end then shrink" workaround.
 */
import type { FlowDynamicPortGroup as WireFlowDynamicPortGroup, FlowNodeDefinition as WireFlowNodeDefinition } from "@battersea/flow";
import { type FlowParameterValues } from "./flow-node-definitions.js";
import { type FlowPortSide } from "./flow-node-ports.js";
import type { FlowStudioWorkspaceState } from "./flow-persistence.js";
interface DynamicPortGroupMatch {
    group: WireFlowDynamicPortGroup;
    index: number;
}
/**
 * Resolves the dynamic port group (and the port's index within it) that owns
 * `portId` on `side`, or `null` when the port is fixed / not variadic.
 */
export declare function resolveDynamicPortGroupMatch(definition: WireFlowNodeDefinition, side: FlowPortSide, portId: string): DynamicPortGroupMatch | null;
/**
 * True when `portId` is a variadic port whose group still has more ports than
 * its minimum, i.e. it can be deleted without violating the count's lower bound.
 */
export declare function canDeleteVariadicPort(options: {
    definition: WireFlowNodeDefinition;
    parameterValues: FlowParameterValues;
    portId: string;
    side: FlowPortSide;
}): boolean;
export declare function deleteVariadicPortInWorkspace(options: {
    definition: WireFlowNodeDefinition;
    nodeId: string;
    portId: string;
    side: FlowPortSide;
    workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState;
export {};

// core/flow-port-move-controls.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Implements non-drag flow node port move-control semantics.
 */
import type { FlowNodeClass as WireFlowNodeClass } from "@battersea/flow";
import { type FlowControllerPortPlacement } from "./flow-controller-port-placement.js";
import type { FlowPortSide, FlowStudioResolvedPort } from "./flow-node-ports.js";
export type FlowPortMoveDirection = "toward-end" | "toward-start";
export interface FlowPortReorderControlState {
    canMoveTowardEnd: boolean;
    canMoveTowardStart: boolean;
    currentIndex: number;
    endIcon: "arrow-down" | "arrow-right";
    endLabel: string;
    kind: "reorder";
    startIcon: "arrow-left" | "arrow-up";
    startLabel: string;
}
export interface FlowPortSwapSideControlState {
    currentIndex: number;
    direction: FlowPortMoveDirection;
    icon: "arrow-down" | "arrow-left" | "arrow-right" | "arrow-up";
    kind: "swap-side";
    label: string;
}
export type FlowPortMoveControlState = FlowPortReorderControlState | FlowPortSwapSideControlState;
export declare function resolveFlowPortMoveControlState(options: {
    controllerPortPlacement?: FlowControllerPortPlacement | null;
    nodeClass?: WireFlowNodeClass | null;
    portId: string;
    ports: readonly FlowStudioResolvedPort[];
    side: FlowPortSide;
}): FlowPortMoveControlState | null;
export declare function resolveFlowPortMoveTargetIndex(options: {
    currentIndex: number;
    direction: FlowPortMoveDirection;
    portCount: number;
}): number | null;

// core/flow-port-order.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Implements saved flow node port ordering and live edge-handle remapping.
 */
import type { FlowNode as WireFlowNode } from "@battersea/flow";
import type { FlowStudioEdge } from "./dataflow-editor-state.js";
import { type FlowPortSide, type FlowStudioResolvedPort } from "./flow-node-ports.js";
export type FlowStudioPortOrder = NonNullable<WireFlowNode["port_order"]>;
export declare function cloneFlowStudioPortOrder(portOrder: FlowStudioPortOrder | null | undefined): FlowStudioPortOrder | undefined;
export declare function hasFlowStudioPortOrder(portOrder: FlowStudioPortOrder | null | undefined): boolean;
export declare function applyFlowPortOrder(ports: readonly FlowStudioResolvedPort[], side: FlowPortSide, portOrder: FlowStudioPortOrder | null | undefined): FlowStudioResolvedPort[];
export declare function normalizeFlowStudioPortOrder(options: {
    actionPorts: readonly FlowStudioResolvedPort[];
    automationPorts: readonly FlowStudioResolvedPort[];
    inputPorts: readonly FlowStudioResolvedPort[];
    outputPorts: readonly FlowStudioResolvedPort[];
    portOrder: FlowStudioPortOrder | null | undefined;
    signalPorts: readonly FlowStudioResolvedPort[];
}): FlowStudioPortOrder | undefined;
export declare function resolveReorderedFlowStudioPortOrder(options: {
    currentPorts: readonly FlowStudioResolvedPort[];
    portOrder: FlowStudioPortOrder | null | undefined;
    side: FlowPortSide;
    activePortId: string;
    overPortId: string;
}): FlowStudioPortOrder | undefined;
export declare function resolveReorderedFlowStudioPortOrderByIndex(options: {
    activePortId: string;
    currentPorts: readonly FlowStudioResolvedPort[];
    portOrder: FlowStudioPortOrder | null | undefined;
    side: FlowPortSide;
    targetIndex: number;
}): FlowStudioPortOrder | undefined;
export declare function remapNodeSideEdgeHandles(options: {
    edges: readonly FlowStudioEdge[];
    nextPorts: readonly FlowStudioResolvedPort[];
    nodeId: string;
    previousPorts: readonly FlowStudioResolvedPort[];
    side: FlowPortSide;
}): FlowStudioEdge[];

// core/flow-port-reorder.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Implements flow node port reorder drag payloads and workspace mutations.
 *
 * For sides that carry one or more dynamic-port groups (e.g. Concatenate's
 * `input-{index}`), reordering renames the underlying port ids so the visual
 * order equals the saved logical order. The Concatenate runtime — and any
 * other handler that sorts dynamic ports by numeric suffix — then honours
 * the drag without us needing a parallel `port_order` field. Fixed-name
 * sides (or sides with no dynamic groups) fall back to the visual-only
 * `port_order` mechanism since their ids are owned by the definition.
 */
import type { DragEndEvent, DragStartEvent } from "@dnd-kit/core";
import type { FlowNodeDefinition as WireFlowNodeDefinition } from "@battersea/flow";
import { type FlowControllerPortPlacement } from "./flow-controller-port-placement.js";
import { type FlowPortSide } from "./flow-node-ports.js";
import type { FlowStudioWorkspaceState } from "./flow-persistence.js";
export interface FlowPortReorderDragData {
    kind: "flow-port-reorder";
    nodeId: string;
    portId: string;
    side: FlowPortSide;
}
export interface FlowPortReorderDropData {
    kind: "flow-port-reorder-target";
    nodeId: string;
    portId: string;
    side: FlowPortSide;
}
export declare function buildFlowPortReorderDragId(nodeId: string, side: FlowPortSide, portId: string): string;
export declare function buildFlowPortReorderDropId(nodeId: string, side: FlowPortSide, portId: string): string;
export declare function readFlowPortReorderDragData(value: unknown): FlowPortReorderDragData | null;
export declare function readFlowPortReorderDropData(value: unknown): FlowPortReorderDropData | null;
export declare function isFlowPortReorderDragStartEvent(event: DragStartEvent): boolean;
export declare function isFlowPortReorderDragEndEvent(event: DragEndEvent): boolean;
export declare function reorderNodePortsInWorkspace(options: {
    activePortId: string;
    definition: WireFlowNodeDefinition;
    nodeId: string;
    previewIndex: number;
    side: FlowPortSide;
    workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState;
export declare function swapNodeControllerPortPlacementInWorkspace(options: {
    definition: WireFlowNodeDefinition;
    nodeId: string;
    workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState;
export declare function setNodeControllerPortPlacementInWorkspace(options: {
    definition: WireFlowNodeDefinition;
    nodeId: string;
    placement: FlowControllerPortPlacement;
    workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState;

// core/flow-ui-state.d.ts
/** Copyright (c) Scott A Dixon */
import type { FlowNameModalMode, FlowSummary as WireFlowSummary } from "../ports.js";
import { type FlowStudioWorkspaceState } from "./flow-persistence.js";
export type FlowSelectionChangeResult = "ignored" | "loaded" | "cancelled" | "failed";
export declare function shouldPromptForFlowName(title: string): boolean;
export declare function getLoadedFlowKey(workspace: FlowStudioWorkspaceState): string;
export declare function getLoadedFlowTitle(workspace: FlowStudioWorkspaceState): string;
export declare function hasLoadedNamedFlow(workspace: FlowStudioWorkspaceState): boolean;
export declare function buildCloneFlowTitleSuggestion(sourceTitle: string, flowSummaries: readonly Pick<WireFlowSummary, "flow_key" | "title">[]): string;
export declare function getFlowNameModalInitialValue(mode: FlowNameModalMode, loadedFlowTitle: string, flowSummaries?: readonly Pick<WireFlowSummary, "flow_key" | "title">[]): string;
export declare function buildWorkspaceForFailedFlowLoad(): FlowStudioWorkspaceState;
export declare function resolveFlowValidationMessage(rawMessage: string): string;
export declare function resolveFlowSelectionChange(options: {
    currentLoadedFlowKey: string;
    isDirty: boolean;
    loadFlow: (flowKey: string) => Promise<boolean>;
    nextFlowKey: string;
    confirmDiscard: () => Promise<boolean>;
}): Promise<FlowSelectionChangeResult>;

// core/parameter-controls.d.ts
/** Copyright (c) Scott A Dixon */
export declare function resolveNumberControlValue(value: unknown, fallback: number): number;
export declare function resolveTextControlValue(value: unknown): string;
export declare function resolveMultiSelectValue(value: unknown): string[];

// graph.d.ts
export * from "./graph/authoring-editable-edge.js";
export * from "./graph/authoring-graph-handles.js";
export * from "./graph/authoring-interactive-gesture-driver.js";
export * from "./graph/authoring-graph-auto-layout-control.js";
export * from "./graph/authoring-graph-edge-bridges.js";
export * from "./graph/authoring-graph-gestures.js";
export * from "./graph/authoring-graph-edge.js";
export * from "./graph/authoring-editable-edge-controller.js";
export * from "./graph/authoring-graph-edge-types.js";
export * from "./graph/authoring-graph-edge-waypoints.js";
export * from "./graph/authoring-graph-canvas.js";
export * from "./graph/layout/types.js";
export * from "./graph/presentation.js";
export * from "./graph/layout/registry.js";

// graph/authoring-editable-edge-controller.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Shares editable-edge controller state across authoring canvases.
 */
import React from "react";
import type { AuthoringGraphBridge, AuthoringGraphPoint, AuthoringGraphWaypoint } from "./authoring-graph-edge-types.js";
export interface UseAuthoringEditableEdgeControllerOptions<TWorkspace, TEdgeKey, TBridgeSelection, TWaypointSelection> {
    clearHostSelection?: (workspace: TWorkspace) => TWorkspace;
    commitChange: (label: string, transform: (workspace: TWorkspace) => TWorkspace) => void;
    createBridgeSelection: (edgeKey: TEdgeKey, bridgeIndex: number) => TBridgeSelection;
    createWaypointSelection: (edgeKey: TEdgeKey, waypointIndex: number) => TWaypointSelection;
    getBridgeCount: (workspace: TWorkspace, edgeKey: TEdgeKey) => number;
    hasBridge: (workspace: TWorkspace, selection: TBridgeSelection) => boolean;
    hasWaypoint: (workspace: TWorkspace, selection: TWaypointSelection) => boolean;
    insertBridge: (workspace: TWorkspace, edgeKey: TEdgeKey, segmentIndex: number, bridge: AuthoringGraphBridge) => TWorkspace;
    insertWaypoint: (workspace: TWorkspace, edgeKey: TEdgeKey, segmentIndex: number, segmentT: number, waypoint: AuthoringGraphWaypoint) => TWorkspace;
    onEdgeInteraction?: () => void;
    removeBridge: (workspace: TWorkspace, selection: TBridgeSelection) => TWorkspace;
    removeWaypoint: (workspace: TWorkspace, selection: TWaypointSelection) => TWorkspace;
    setWorkspace: React.Dispatch<React.SetStateAction<TWorkspace>>;
    updateBridgeGap: (workspace: TWorkspace, edgeKey: TEdgeKey, bridgeIndex: number, gap: number) => TWorkspace;
    updateBridgePosition: (workspace: TWorkspace, edgeKey: TEdgeKey, bridgeIndex: number, segmentIndex: number, t: number) => TWorkspace;
    updateWaypointHandle: (workspace: TWorkspace, edgeKey: TEdgeKey, waypointIndex: number, handleKind: "inHandle" | "outHandle", independent: boolean, position: AuthoringGraphPoint) => TWorkspace;
    updateWaypointPosition: (workspace: TWorkspace, edgeKey: TEdgeKey, waypointIndex: number, position: AuthoringGraphPoint) => TWorkspace;
    workspace: TWorkspace;
}
export interface AuthoringEditableEdgeControllerState<TEdgeKey, TBridgeSelection, TWaypointSelection> {
    clearSelectedEdgeBridge: () => void;
    clearSelectedEdgeAffordances: () => void;
    clearSelectedEdgeWaypoint: () => void;
    handleInsertEdgeBridge: (edgeKey: TEdgeKey, segmentIndex: number, bridge: AuthoringGraphBridge) => void;
    handleInsertEdgeWaypoint: (edgeKey: TEdgeKey, segmentIndex: number, segmentT: number, waypoint: AuthoringGraphWaypoint) => void;
    handleSelectEdgeBridge: (edgeKey: TEdgeKey, bridgeIndex: number) => void;
    handleSelectEdgeWaypoint: (edgeKey: TEdgeKey, waypointIndex: number) => void;
    handleUpdateEdgeBridgeGap: (edgeKey: TEdgeKey, bridgeIndex: number, gap: number) => void;
    handleUpdateEdgeBridgePosition: (edgeKey: TEdgeKey, bridgeIndex: number, segmentIndex: number, t: number) => void;
    handleUpdateEdgeWaypointHandle: (edgeKey: TEdgeKey, waypointIndex: number, handleKind: "inHandle" | "outHandle", independent: boolean, position: AuthoringGraphPoint) => void;
    handleUpdateEdgeWaypointPosition: (edgeKey: TEdgeKey, waypointIndex: number, position: AuthoringGraphPoint) => void;
    removeSelectedEdgeBridge: () => void;
    removeSelectedEdgeWaypoint: () => void;
    selectedEdgeBridge: TBridgeSelection | null;
    selectedEdgeWaypoint: TWaypointSelection | null;
}
export declare function useAuthoringEditableEdgeController<TWorkspace, TEdgeKey, TBridgeSelection, TWaypointSelection>(options: UseAuthoringEditableEdgeControllerOptions<TWorkspace, TEdgeKey, TBridgeSelection, TWaypointSelection>): AuthoringEditableEdgeControllerState<TEdgeKey, TBridgeSelection, TWaypointSelection>;

// graph/authoring-editable-edge.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Defines the shared editable-edge contract used by authoring canvases.
 */
import React from "react";
import { type Edge, type EdgeProps } from "@xyflow/react";
import { eventTargetMatchesWaypointHoverSurface, stopWaypointSurfacePropagation, type AuthoringGraphEdgeTheme } from "./authoring-graph-edge.js";
import type { AuthoringGraphBridge, AuthoringGraphPoint, AuthoringGraphWaypoint } from "./authoring-graph-edge-types.js";
import type { AuthoringGraphInteractiveGestureDriver } from "./authoring-graph-gestures.js";
export { eventTargetMatchesWaypointHoverSurface, stopWaypointSurfacePropagation, };
export type AuthoringEditableEdgeTheme = AuthoringGraphEdgeTheme;
export interface AuthoringEditableEdgeDataShape extends Record<string, unknown> {
    addBridgeLabel?: string;
    addWaypointLabel?: string;
    bridges?: AuthoringGraphBridge[];
    curveOffsetPx?: number;
    directionMarker?: boolean;
    directionMarkerPlacement?: "target-boundary" | "visible-interval";
    directionMarkerTargetBorderRadiusPx?: number;
    gestureDriver?: AuthoringGraphInteractiveGestureDriver;
    onInsertBridge?: (segmentIndex: number, bridge: AuthoringGraphBridge) => void;
    onInsertWaypoint?: (segmentIndex: number, segmentT: number, waypoint: AuthoringGraphWaypoint) => void;
    onSelectBridge?: (bridgeIndex: number) => void;
    onSelectWaypoint?: (waypointIndex: number) => void;
    onUpdateBridgeGap?: (bridgeIndex: number, gap: number) => void;
    onUpdateBridgePosition?: (bridgeIndex: number, segmentIndex: number, t: number) => void;
    onUpdateWaypointHandle?: (waypointIndex: number, handleKind: "inHandle" | "outHandle", independent: boolean, position: AuthoringGraphPoint) => void;
    onUpdateWaypointPosition?: (waypointIndex: number, position: AuthoringGraphPoint) => void;
    selectedBridgeIndex?: number;
    selectedWaypointIndex?: number;
    showSecondaryPath?: boolean;
    theme?: AuthoringEditableEdgeTheme;
    waypoints?: AuthoringGraphWaypoint[];
}
export type AuthoringEditableEdgeData<TExtra extends Record<string, unknown> = Record<string, never>> = TExtra & AuthoringEditableEdgeDataShape;
export declare function AuthoringEditableEdgeView(props: EdgeProps<Edge<AuthoringEditableEdgeData>>): React.JSX.Element;

// graph/authoring-graph-auto-layout-control.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * The auto-layout control rendered inside the shared canvas controls cluster.
 * It exposes a one-shot "apply" action plus an engine picker popover, replacing
 * the old persistent on/off toggle. Engine selection and the apply action are
 * owned by the feature tab (see the controller contract below); this component
 * only renders the picker and reports intent.
 */
import React from "react";
import type { LayoutEngineDescriptor } from "./layout/types.js";
export type AuthoringGraphAutoLayoutStatus = "idle" | "running";
export interface AuthoringGraphAutoLayoutController {
    /** Engines offered for this canvas, in display order. */
    engines: readonly LayoutEngineDescriptor[];
    /** Currently selected engine id; applied when the user presses apply. */
    activeEngineId: string;
    /** Selects a different engine without applying it. */
    onSelectEngine: (engineId: string) => void;
    /** Computes and commits a layout with the given engine. */
    onApply: (engineId: string) => void;
    /** Current values for each engine's parameters, keyed by engine id then parameter id. */
    parameterValues?: Readonly<Record<string, Readonly<Record<string, number>>>>;
    /** Updates one engine parameter value (does not apply); the tab persists it. */
    onParameterChange?: (engineId: string, parameterId: string, value: number) => void;
    /** `running` while a layout is being computed; disables the apply action. */
    status?: AuthoringGraphAutoLayoutStatus;
    /** Disables the whole control (e.g. while the document is loading). */
    disabled?: boolean;
    /** Accessible label prefix; defaults to "Auto layout". */
    label?: string;
    /** Hides the control entirely when false. */
    visible?: boolean;
}
interface AuthoringGraphAutoLayoutControlProps {
    controller?: AuthoringGraphAutoLayoutController;
    paneRef: React.RefObject<HTMLDivElement | null>;
}
export declare function AuthoringGraphAutoLayoutControl({ controller, paneRef, }: AuthoringGraphAutoLayoutControlProps): React.JSX.Element | null;
export {};

// graph/authoring-graph-canvas.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Hosts the shared React Flow substrate used by editor authoring canvases.
 */
import React from "react";
import { type GraphPresentation } from "./presentation.js";
import { type Connection, type ControlProps, type DefaultEdgeOptions, type Edge, type EdgeChange, type EdgeMouseHandler, type EdgeTypes, type FinalConnectionState, type Node, type NodeChange, type OnMove, type OnNodeDrag, type NodeMouseHandler, type NodeTypes, type OnReconnect, type ReactFlowInstance } from "@xyflow/react";
import { type AuthoringGraphAutoLayoutController } from "./authoring-graph-auto-layout-control.js";
export type { AuthoringGraphAutoLayoutController, AuthoringGraphAutoLayoutStatus, } from "./authoring-graph-auto-layout-control.js";
export interface AuthoringGraphCanvasReactFlowProps<TNode extends Node = Node, TEdge extends Edge = Edge> {
    ariaLabel?: string;
    connectionLineStyle?: React.CSSProperties;
    defaultEdgeOptions?: DefaultEdgeOptions;
    edgesReconnectable?: boolean;
    fitView?: boolean;
    /** Furthest the user can zoom out. React Flow defaults to 0.5. */
    minZoom?: number;
    /** Closest the user can zoom in. React Flow defaults to 2. */
    maxZoom?: number;
    /**
     * Only mount nodes/edges inside (or near) the viewport. Essential for large or
     * spread-out graphs (e.g. the globe-spanning Geographic layout) — without it
     * React Flow renders every node and every full-length edge path off-screen.
     */
    onlyRenderVisibleElements?: boolean;
    nodesDraggable?: boolean;
    onEdgeClick?: EdgeMouseHandler<TEdge>;
    onMove?: OnMove;
    onReconnect?: OnReconnect<TEdge>;
}
export interface AuthoringGraphCanvasViewportSize {
    height: number;
    width: number;
}
export interface AuthoringGraphCanvasProps<TNode extends Node = Node, TEdge extends Edge = Edge> {
    presentation?: GraphPresentation;
    canvasClassName: string;
    autoLayoutController?: AuthoringGraphAutoLayoutController;
    controlsProps?: false | ControlProps;
    dropzoneId: string;
    edgeTypes: EdgeTypes;
    edges: readonly TEdge[];
    flowInstanceRef: React.MutableRefObject<ReactFlowInstance<TNode, TEdge> | null>;
    isValidConnection?: (connection: Connection | TEdge) => boolean;
    nodeTypes: NodeTypes;
    nodes: readonly TNode[];
    onConnect?: (connection: Connection) => void;
    onConnectEnd?: (event: MouseEvent | TouchEvent, connectionState: FinalConnectionState) => void;
    onEdgesChange?: (changes: EdgeChange<TEdge>[]) => void;
    onFlowInit?: (instance: ReactFlowInstance<TNode, TEdge>) => void;
    onNodeDrag?: OnNodeDrag<TNode>;
    onNodeDragStart?: OnNodeDrag<TNode>;
    onNodeDragStop?: OnNodeDrag<TNode>;
    onNodeClick?: NodeMouseHandler<TNode>;
    onNodeContextMenu?: (event: React.MouseEvent, node: TNode) => void;
    onNodesChange?: (changes: NodeChange<TNode>[]) => void;
    onPaneClick?: () => void;
    onViewportSizeChange?: (size: AuthoringGraphCanvasViewportSize) => void;
    overlay?: React.ReactNode;
    paneClassName: string;
    paneRef?: React.RefCallback<HTMLDivElement> | React.MutableRefObject<HTMLDivElement | null> | null;
    reactFlowProps?: AuthoringGraphCanvasReactFlowProps<TNode, TEdge>;
}
export declare function AuthoringGraphCanvas<TNode extends Node = Node, TEdge extends Edge = Edge>({ presentation, canvasClassName, autoLayoutController, controlsProps, dropzoneId, edgeTypes, edges, flowInstanceRef, isValidConnection, nodeTypes, nodes, onConnect, onConnectEnd, onEdgesChange, onFlowInit, onNodeDrag, onNodeDragStart, onNodeDragStop, onNodeClick, onNodeContextMenu, onNodesChange, onPaneClick, onViewportSizeChange, overlay, paneClassName, paneRef: externalPaneRef, reactFlowProps, }: AuthoringGraphCanvasProps<TNode, TEdge>): React.JSX.Element;

// graph/authoring-graph-edge-bridges.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Builds and mutates manual edge bridge geometry for authoring-graph edges.
 */
import type { AuthoringGraphBridge, AuthoringGraphEdgeLayout, AuthoringGraphPoint } from "./authoring-graph-edge-types.js";
import type { AuthoringGraphWaypointSegment } from "./authoring-graph-edge-waypoints.js";
export interface AuthoringGraphBridgeLocation {
    segmentIndex: number;
    t: number;
}
export interface AuthoringGraphInsertionInterval {
    endT: number;
    segmentIndex: number;
    startT: number;
}
export interface AuthoringGraphBridgeGapInterval {
    endT: number;
    segmentIndex: number;
    startT: number;
}
export interface ResolvedAuthoringGraphBridgeGeometry {
    center: AuthoringGraphPoint;
    endCapEnd: AuthoringGraphPoint;
    endCapStart: AuthoringGraphPoint;
    gapEnd: AuthoringGraphPoint;
    gapStart: AuthoringGraphPoint;
    normal: AuthoringGraphPoint;
    sizeHandle: AuthoringGraphPoint;
    startCapEnd: AuthoringGraphPoint;
    startCapStart: AuthoringGraphPoint;
    tangent: AuthoringGraphPoint;
}
export interface ResolvedAuthoringGraphDirectionMarkerGeometry {
    position: AuthoringGraphPoint;
    rotationDegrees: number;
    tangent: AuthoringGraphPoint;
}
export interface AuthoringGraphDirectionMarkerBoundary {
    borderRadiusPx?: number;
    center: AuthoringGraphPoint;
    height: number;
    width: number;
}
export declare function createBridgeForInterval(interval: AuthoringGraphInsertionInterval): AuthoringGraphBridge;
export declare function createBridgeForSegment(segmentIndex: number): AuthoringGraphBridge;
export declare function insertBridgeIntoLayout(layout: AuthoringGraphEdgeLayout | undefined, bridge: AuthoringGraphBridge): AuthoringGraphEdgeLayout;
export declare function moveBridgeInLayout(layout: AuthoringGraphEdgeLayout | undefined, bridgeIndex: number, location: AuthoringGraphBridgeLocation): AuthoringGraphEdgeLayout;
export declare function resizeBridgeInLayout(layout: AuthoringGraphEdgeLayout | undefined, bridgeIndex: number, gap: number): AuthoringGraphEdgeLayout;
export declare function removeBridgeFromLayout(layout: AuthoringGraphEdgeLayout | undefined, bridgeIndex: number): AuthoringGraphEdgeLayout;
export declare function resolveNearestBridgeLocation(segments: readonly AuthoringGraphWaypointSegment[], point: AuthoringGraphPoint): AuthoringGraphBridgeLocation | null;
export declare function resolveBridgeGeometry(segments: readonly AuthoringGraphWaypointSegment[], bridge: AuthoringGraphBridge): ResolvedAuthoringGraphBridgeGeometry | null;
export declare function buildVisibleEdgePathWithBridges(segments: readonly AuthoringGraphWaypointSegment[], bridges: readonly AuthoringGraphBridge[]): string;
export declare function resolveBridgeGapIntervals(segments: readonly AuthoringGraphWaypointSegment[], bridges: readonly AuthoringGraphBridge[]): AuthoringGraphBridgeGapInterval[];
export declare function buildEdgeInsertionIntervals(segments: readonly AuthoringGraphWaypointSegment[], bridges: readonly AuthoringGraphBridge[]): AuthoringGraphInsertionInterval[];
export declare function resolveInsertionIntervalMidpoint(segments: readonly AuthoringGraphWaypointSegment[], interval: AuthoringGraphInsertionInterval): AuthoringGraphPoint | null;
export declare function resolveDirectionMarkerGeometry(segments: readonly AuthoringGraphWaypointSegment[], bridges: readonly AuthoringGraphBridge[]): ResolvedAuthoringGraphDirectionMarkerGeometry | null;
export declare function resolveTargetBoundaryDirectionMarkerGeometry(segments: readonly AuthoringGraphWaypointSegment[], boundary: AuthoringGraphDirectionMarkerBoundary): ResolvedAuthoringGraphDirectionMarkerGeometry | null;
export declare function resolveNearestInsertionInterval(segments: readonly AuthoringGraphWaypointSegment[], bridges: readonly AuthoringGraphBridge[], point: AuthoringGraphPoint): AuthoringGraphInsertionInterval | null;
export declare function updateBridgeIndicesForWaypointInsertion(bridges: readonly AuthoringGraphBridge[] | undefined, insertedSegmentIndex: number, insertedT: number): AuthoringGraphBridge[] | undefined;
export declare function updateBridgeIndicesForWaypointRemoval(bridges: readonly AuthoringGraphBridge[] | undefined, removedWaypointIndex: number): AuthoringGraphBridge[] | undefined;
export declare function resolveBridgeGapFromHandle(geometry: ResolvedAuthoringGraphBridgeGeometry, handlePosition: AuthoringGraphPoint): number;

// graph/authoring-graph-edge-types.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Declares shared editable-edge types for authoring-graph canvases.
 */
export interface AuthoringGraphPoint {
    x: number;
    y: number;
}
export interface AuthoringGraphWaypoint {
    inHandle: AuthoringGraphPoint;
    outHandle: AuthoringGraphPoint;
    position: AuthoringGraphPoint;
}
export interface AuthoringGraphBridge {
    gap: number;
    segmentIndex: number;
    t: number;
}
export interface AuthoringGraphEdgeLayout {
    bridges?: AuthoringGraphBridge[];
    waypoints?: AuthoringGraphWaypoint[];
}
export interface AuthoringGraphEdgeSelection {
    bridgeIndex?: number;
    waypointIndex?: number;
}

// graph/authoring-graph-edge-waypoints.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Builds and mutates editable bezier waypoint geometry for authoring-graph edges.
 */
import { Position } from "@xyflow/react";
import type { AuthoringGraphEdgeLayout, AuthoringGraphPoint, AuthoringGraphWaypoint } from "./authoring-graph-edge-types.js";
export interface AuthoringGraphWaypointSegment {
    controlA: AuthoringGraphPoint;
    controlB: AuthoringGraphPoint;
    end: AuthoringGraphPoint;
    start: AuthoringGraphPoint;
}
export interface BuildAuthoringGraphWaypointSegmentsOptions {
    source: AuthoringGraphPoint;
    sourcePosition: Position;
    target: AuthoringGraphPoint;
    targetPosition: Position;
    waypoints: readonly AuthoringGraphWaypoint[];
}
export declare function buildAuthoringGraphWaypointSegments(options: BuildAuthoringGraphWaypointSegmentsOptions): AuthoringGraphWaypointSegment[];
export declare function buildAuthoringGraphWaypointPath(options: BuildAuthoringGraphWaypointSegmentsOptions): string;
export declare function evaluateAuthoringGraphCubicBezierPoint(segment: AuthoringGraphWaypointSegment, t: number): AuthoringGraphPoint;
export declare function resolveNearestWaypointSegmentIndex(segments: readonly AuthoringGraphWaypointSegment[], point: AuthoringGraphPoint): number | null;
export declare function createAuthoringGraphWaypointForSegment(segment: AuthoringGraphWaypointSegment): AuthoringGraphWaypoint;
export declare function createAuthoringGraphWaypointForSegmentAt(segment: AuthoringGraphWaypointSegment, t: number): AuthoringGraphWaypoint;
export declare function insertWaypointIntoLayout(layout: AuthoringGraphEdgeLayout | undefined, segmentIndex: number, waypoint: AuthoringGraphWaypoint, segmentT?: number): AuthoringGraphEdgeLayout;
export declare function moveWaypointInLayout(layout: AuthoringGraphEdgeLayout | undefined, waypointIndex: number, position: AuthoringGraphPoint): AuthoringGraphEdgeLayout;
export declare function moveWaypointHandleInLayout(layout: AuthoringGraphEdgeLayout | undefined, waypointIndex: number, handleKind: "inHandle" | "outHandle", independent: boolean, position: AuthoringGraphPoint): AuthoringGraphEdgeLayout;
export declare function removeWaypointFromLayout(layout: AuthoringGraphEdgeLayout | undefined, waypointIndex: number): AuthoringGraphEdgeLayout;

// graph/authoring-graph-edge.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Renders the shared interactive edge layer used by authoring-graph canvases.
 */
import React from "react";
import { type Edge, type EdgeProps } from "@xyflow/react";
import type { AuthoringGraphBridge, AuthoringGraphEdgeLayout, AuthoringGraphEdgeSelection, AuthoringGraphPoint, AuthoringGraphWaypoint } from "./authoring-graph-edge-types.js";
import { type AuthoringGraphInteractiveGestureDriver } from "./authoring-graph-gestures.js";
export interface AuthoringGraphEdgeTheme {
    addButtonClassName?: string;
    addButtonShellClassName?: string;
    bridgeButtonClassName?: string;
    bridgeButtonGlyphClassName?: string;
    bridgeCapClassName?: string;
    bridgeDragHandleClassName?: string;
    bridgeGapHandleClassName?: string;
    bridgeGapLineClassName?: string;
    bridgeGroupClassName?: string;
    bridgeHitTargetClassName?: string;
    bridgeSelectedClassName?: string;
    directionMarkerClassName?: string;
    directionMarkerGlyphClassName?: string;
    interactionClassName?: string;
    pathClassName?: string;
    secondaryPathClassName?: string;
    selectionOutlineClassName?: string;
    waypointAnchorClassName?: string;
    waypointGroupClassName?: string;
    waypointHandleClassName?: string;
    waypointHandleInClassName?: string;
    waypointHandleLineClassName?: string;
    waypointHandleOutClassName?: string;
    waypointSelectedClassName?: string;
}
export interface AuthoringGraphEdgeProps extends EdgeProps<Edge> {
    addBridgeLabel?: string;
    addWaypointLabel?: string;
    curveOffsetPx?: number;
    directionMarker?: boolean;
    directionMarkerPlacement?: "target-boundary" | "visible-interval";
    directionMarkerTargetBorderRadiusPx?: number;
    gestureDriver?: AuthoringGraphInteractiveGestureDriver;
    layout?: AuthoringGraphEdgeLayout;
    onInsertBridge?: (segmentIndex: number, bridge: AuthoringGraphBridge) => void;
    onInsertWaypoint?: (segmentIndex: number, segmentT: number, waypoint: AuthoringGraphWaypoint) => void;
    onSelectBridge?: (bridgeIndex: number) => void;
    onSelectWaypoint?: (waypointIndex: number) => void;
    onUpdateBridgeGap?: (bridgeIndex: number, gap: number) => void;
    onUpdateBridgePosition?: (bridgeIndex: number, segmentIndex: number, t: number) => void;
    onUpdateWaypointHandle?: (waypointIndex: number, handleKind: "inHandle" | "outHandle", independent: boolean, position: AuthoringGraphPoint) => void;
    onUpdateWaypointPosition?: (waypointIndex: number, position: AuthoringGraphPoint) => void;
    selection?: AuthoringGraphEdgeSelection;
    showSecondaryPath?: boolean;
    theme?: AuthoringGraphEdgeTheme;
}
export declare function eventTargetMatchesWaypointHoverSurface(target: EventTarget | null, selector: string): boolean;
export declare function stopWaypointSurfacePropagation(event: Pick<React.SyntheticEvent, "preventDefault" | "stopPropagation" | "nativeEvent">): void;
export declare function AuthoringGraphEdge({ addBridgeLabel, addWaypointLabel, curveOffsetPx, directionMarker, directionMarkerPlacement, directionMarkerTargetBorderRadiusPx, gestureDriver, layout, onInsertBridge, onInsertWaypoint, onSelectBridge, onSelectWaypoint, onUpdateBridgeGap, onUpdateBridgePosition, onUpdateWaypointHandle, onUpdateWaypointPosition, selection, showSecondaryPath, theme, ...props }: AuthoringGraphEdgeProps): React.JSX.Element;
export declare function findReactFlowNodeElement(canvas: HTMLElement | null, nodeId: string): HTMLElement | null;

// graph/authoring-graph-gestures.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Declares shared interactive gesture lifecycle contracts for authoring-graph canvases.
 */
export type AuthoringGraphGestureLabel = "Move node" | "Move bridge" | "Move waypoint" | "Move waypoint handle" | "Resize bridge";
export interface AuthoringGraphInteractiveGestureDriver {
    beginInteractiveGesture: (label: AuthoringGraphGestureLabel) => void;
    cancelInteractiveGesture: () => void;
    commitInteractiveGesture: () => void;
}
export interface AuthoringGraphInteractiveGestureSession {
    beginGesture: (driver: AuthoringGraphInteractiveGestureDriver | undefined, label: AuthoringGraphGestureLabel) => void;
    cancelGesture: (driver: AuthoringGraphInteractiveGestureDriver | undefined) => void;
    commitGesture: (driver: AuthoringGraphInteractiveGestureDriver | undefined) => void;
    getActiveLabel: () => AuthoringGraphGestureLabel | null;
}
export type AuthoringGraphEdgeGestureHandleKind = "anchor" | "bridge" | "bridgeGap" | "inHandle" | "outHandle";
export declare function resolveAuthoringGraphEdgeGestureLabel(handleKind: AuthoringGraphEdgeGestureHandleKind): AuthoringGraphGestureLabel;
export declare function createAuthoringGraphInteractiveGestureSession(): AuthoringGraphInteractiveGestureSession;

// graph/authoring-graph-handles.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Declares shared handle descriptors and structural connection helpers for authoring graphs.
 */
import React from "react";
import { Position } from "@xyflow/react";
export type AuthoringGraphHandleSide = "top" | "right" | "bottom" | "left";
export type AuthoringGraphHandleDirection = "source" | "target";
export interface AuthoringGraphHandleDescriptor<TFamily extends string = string> {
    anchorOffset?: number;
    data?: Record<string, unknown>;
    direction: AuthoringGraphHandleDirection;
    family: TFamily;
    handleId: string;
    label?: string;
    orderIndex?: number;
    side: AuthoringGraphHandleSide;
}
export interface AuthoringGraphResolvedConnection<TFamily extends string = string> {
    family: TFamily;
    source: string;
    sourceHandleId: string;
    sourceHandle: AuthoringGraphHandleDescriptor<TFamily>;
    target: string;
    targetHandleId: string;
    targetHandle: AuthoringGraphHandleDescriptor<TFamily>;
}
export type AuthoringGraphConnectionInspectionFailure = "family_mismatch" | "missing_endpoints" | "missing_source_handle" | "missing_target_handle" | "self_loop" | "source_direction_mismatch" | "target_direction_mismatch";
export type AuthoringGraphConnectionInspection<TFamily extends string = string> = {
    connection: AuthoringGraphResolvedConnection<TFamily>;
    valid: true;
} | {
    reason: AuthoringGraphConnectionInspectionFailure;
    valid: false;
};
export interface AuthoringGraphHandleIndex<TFamily extends string = string> {
    byDirection: ReadonlyMap<AuthoringGraphHandleDirection, readonly AuthoringGraphHandleDescriptor<TFamily>[]>;
    byFamily: ReadonlyMap<TFamily, readonly AuthoringGraphHandleDescriptor<TFamily>[]>;
    byId: ReadonlyMap<string, AuthoringGraphHandleDescriptor<TFamily>>;
    bySide: ReadonlyMap<AuthoringGraphHandleSide, readonly AuthoringGraphHandleDescriptor<TFamily>[]>;
}
export interface AuthoringGraphHandleProps<TFamily extends string = string> {
    className?: string;
    descriptor: AuthoringGraphHandleDescriptor<TFamily>;
    isConnectable?: boolean;
    onClick?: React.MouseEventHandler<HTMLDivElement>;
    style?: React.CSSProperties;
    title?: string;
    [dataAttribute: `data-${string}`]: string | number | boolean | undefined;
}
export interface AuthoringGraphConnectionLike {
    source?: string | null;
    sourceHandle?: string | null;
    target?: string | null;
    targetHandle?: string | null;
}
export interface InspectAuthoringGraphConnectionOptions<TFamily extends string = string> {
    allowSelfLoops?: boolean;
    connection: AuthoringGraphConnectionLike;
    defaultSourceHandleId?: string;
    defaultTargetHandleId?: string;
    sourceHandles: readonly AuthoringGraphHandleDescriptor<TFamily>[];
    targetHandles: readonly AuthoringGraphHandleDescriptor<TFamily>[];
}
export declare function indexAuthoringGraphHandles<TFamily extends string = string>(descriptors: readonly AuthoringGraphHandleDescriptor<TFamily>[]): AuthoringGraphHandleIndex<TFamily>;
export declare function inspectAuthoringGraphConnection<TFamily extends string = string>(options: InspectAuthoringGraphConnectionOptions<TFamily>): AuthoringGraphConnectionInspection<TFamily>;
export declare function resolveAuthoringGraphConnection<TFamily extends string = string>(options: InspectAuthoringGraphConnectionOptions<TFamily>): AuthoringGraphResolvedConnection<TFamily> | null;
export declare function getAuthoringGraphHandlePosition(side: AuthoringGraphHandleSide): Position;
export declare function AuthoringGraphHandle<TFamily extends string = string>({ className, descriptor, isConnectable, onClick, style, title, ...dataAttributes }: AuthoringGraphHandleProps<TFamily>): React.JSX.Element;

// graph/authoring-interactive-gesture-driver.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Adapts shared authoring-graph gesture lifecycles onto feature-specific undo.
 */
import type { AuthoringGraphGestureLabel, AuthoringGraphInteractiveGestureDriver } from "./authoring-graph-gestures.js";
export declare function createAuthoringInteractiveGestureDriver<TSnapshot>(input: {
    capture: () => TSnapshot;
    commit: (label: AuthoringGraphGestureLabel, snapshot: TSnapshot) => void;
}): AuthoringGraphInteractiveGestureDriver;

// graph/layout/dagre-engine.d.ts
import type { LayoutEngine } from "./types.js";
export declare function createDagreEngine(): LayoutEngine;

// graph/layout/elk-graph.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Pure translation between the neutral `LayoutGraph` contract and ELK's graph
 * JSON. Kept free of any `elkjs` import so it can be unit-tested in Node without
 * loading the layout engine.
 */
import type { LayoutGraph, LayoutPositions } from "./types.js";
export type ElkAlgorithm = "layered" | "mrtree";
/** Minimal shape of the ELK graph we build; mirrors elkjs `ElkNode`. */
export interface ElkGraphNode {
    id: string;
    width?: number;
    height?: number;
    x?: number;
    y?: number;
    children?: ElkGraphNode[];
}
export interface ElkGraph {
    id: string;
    layoutOptions: Record<string, string>;
    children: ElkGraphNode[];
    edges: Array<{
        id: string;
        sources: [string];
        targets: [string];
    }>;
}
export interface BuildElkGraphOptions {
    algorithm: ElkAlgorithm;
    direction?: "right" | "down";
}
/**
 * Builds an ELK graph for the supplied neutral graph. `mrtree` lays the
 * containment hierarchy out as a tidy tree; `layered` produces a directional,
 * crossing-minimised flow. Only `containment` and `flow` edges steer the
 * hierarchy; `link` cross-references are still passed to ELK so layered routing
 * accounts for them, but they never define tree parentage.
 */
export declare function buildElkGraph(graph: LayoutGraph, options: BuildElkGraphOptions): ElkGraph;
/**
 * Reads absolute top-left positions back out of a laid-out ELK graph. ELK
 * returns coordinates relative to the parent; our graph is flat (single level),
 * so child coordinates are already absolute.
 */
export declare function readElkPositions(result: {
    children?: ElkGraphNode[];
}): LayoutPositions;

// graph/layout/geo-engine.d.ts
import type { LayoutEngine, LayoutGraph, LayoutPositions, LayoutRunOptions } from "./types.js";
export declare function createGeographicEngine(id: string): LayoutEngine;
/**
 * The compact geographic engine. Same projection and the same faithful scale
 * *within* each cluster — local distances still read true — but the vast empty
 * gulfs *between* clusters are collapsed so every distinct cluster fits a single
 * viewport. For a story where the real scale of the places is interesting yet
 * the oceans between regions are just dead space.
 */
export declare function createCompactGeographicEngine(id: string): LayoutEngine;
/** Faithful map: real distances; far outliers compressed to the rim. */
export declare function runGeographicLayout(graph: LayoutGraph, options?: LayoutRunOptions): LayoutPositions;
/** Compact map: faithful within each cluster; the gulfs between clusters collapsed. */
export declare function runCompactGeographicLayout(graph: LayoutGraph, options?: LayoutRunOptions): LayoutPositions;

// graph/layout/gravity-engine.d.ts
import type { LayoutEngine, LayoutGraph, LayoutPositions, LayoutRunOptions } from "./types.js";
export declare function createGravityEngine(id: string): LayoutEngine;
/** Pure simulation core — no async, no DOM — kept separate so it can be unit-tested directly. */
export declare function runGravityLayout(graph: LayoutGraph, options?: LayoutRunOptions): LayoutPositions;

// graph/layout/registry.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * The auto-layout engine registry. Holds UI-facing descriptors (safe to import
 * anywhere, including Node tests and the main bundle) and a lazy loader that
 * dynamically imports the heavy engine implementation only when a layout is
 * actually applied.
 */
import type { LayoutEngine, LayoutEngineDescriptor } from "./types.js";
export declare const DAGRE_ENGINE_ID = "dagre";
export declare const DAGRE_ENGINE_DESCRIPTOR: LayoutEngineDescriptor;
export declare const ELK_TREE_ENGINE_ID = "elk-tree";
export declare const ELK_LAYERED_ENGINE_ID = "elk-layered";
export declare const GRAVITY_ENGINE_ID = "gravity";
export declare const GEOGRAPHIC_ENGINE_ID = "geographic";
export declare const GEOGRAPHIC_COMPACT_ENGINE_ID = "geographic-compact";
export declare const ELK_TREE_ENGINE_DESCRIPTOR: LayoutEngineDescriptor;
export declare const ELK_LAYERED_ENGINE_DESCRIPTOR: LayoutEngineDescriptor;
export declare const GRAVITY_ENGINE_DESCRIPTOR: LayoutEngineDescriptor;
export declare const GEOGRAPHIC_ENGINE_DESCRIPTOR: LayoutEngineDescriptor;
export declare const GEOGRAPHIC_COMPACT_ENGINE_DESCRIPTOR: LayoutEngineDescriptor;
/** All engines known to the registry, in display order. */
export declare const LAYOUT_ENGINE_DESCRIPTORS: readonly LayoutEngineDescriptor[];
export declare function isLayoutEngineId(id: string): boolean;
export declare function resolveLayoutEngineDescriptor(id: string): LayoutEngineDescriptor | undefined;
/**
 * Picks the supplied engine id when known, otherwise falls back to the first
 * registered engine. Keeps persisted/unknown ids from breaking the picker.
 */
export declare function resolveLayoutEngineId(id: string | undefined, available: readonly LayoutEngineDescriptor[]): string;
/**
 * Loads a layout engine on demand.
 */
export declare function loadLayoutEngine(id: string): Promise<LayoutEngine>;

// graph/layout/types.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Declares the tab-agnostic graph contract that authoring-graph auto-layout
 * engines operate on. Feature tabs (World, Dataflow) adapt their domain graphs
 * into a `LayoutGraph`; engines return absolute node positions that the tab
 * commits into its own persisted layout.
 */
/** EPSG:4326 (WGS 84) coordinate in decimal degrees. */
export interface LayoutGeoCoordinate {
    latitude: number;
    longitude: number;
}
/**
 * A single layout node. Sizes are in canvas pixels and drive collision/spacing.
 *
 * `geo` is the one piece of engine-specific *input* the neutral graph carries:
 * the adapter populates it for geolocated nodes, the geographic engine projects
 * it, and every other engine ignores it. Optional per-engine inputs ride here so
 * a single adapter can feed every engine.
 */
export interface LayoutNode {
    id: string;
    width: number;
    height: number;
    geo?: LayoutGeoCoordinate;
}
/**
 * Edge semantics. `containment` is the hierarchy spine (parent → child),
 * `link` is a cross-reference, and `flow` is a directed dataflow connection.
 * Engines may weight these differently (e.g. gravity springs harder on links).
 */
export type LayoutEdgeKind = "containment" | "link" | "flow";
export interface LayoutEdge {
    id: string;
    source: string;
    target: string;
    kind: LayoutEdgeKind;
}
/**
 * The neutral graph handed to an engine. `roots` names the nodes an engine
 * should treat as hierarchy entry points (e.g. the World root place); engines
 * that ignore hierarchy may disregard it.
 */
export interface LayoutGraph {
    nodes: readonly LayoutNode[];
    edges: readonly LayoutEdge[];
    roots: readonly string[];
}
/** Top-left canvas coordinate for a laid-out node, matching React Flow positions. */
export interface LayoutPosition {
    x: number;
    y: number;
}
export type LayoutPositions = Record<string, LayoutPosition>;
export interface LayoutRunOptions {
    /** Primary flow direction for directional engines. Defaults per engine. */
    direction?: "right" | "down";
    /**
     * Engine parameter values, keyed by parameter id (see
     * `LayoutEngineDescriptor.parameters`). Absent values fall back to each
     * parameter's `defaultValue`; engines must tolerate a missing map.
     */
    parameters?: Readonly<Record<string, number>>;
    /** Aborts an in-flight run; engines should reject when signalled. */
    signal?: AbortSignal;
}
/**
 * A continuously-adjustable numeric engine parameter — rendered as a slider in
 * the auto-layout control and threaded into `run` via `LayoutRunOptions`. The
 * `kind` discriminant leaves room for boolean/enum parameters later.
 */
export interface LayoutEngineNumberParameter {
    kind: "number";
    id: string;
    label: string;
    description?: string;
    min: number;
    max: number;
    step: number;
    defaultValue: number;
}
export type LayoutEngineParameter = LayoutEngineNumberParameter;
/** A computed layout engine, loaded on demand from the registry. */
export interface LayoutEngine {
    id: string;
    run(graph: LayoutGraph, options?: LayoutRunOptions): Promise<LayoutPositions>;
}
/**
 * UI-facing engine metadata. Carried by the canvas auto-layout controller so the
 * picker can render an engine list without importing any engine implementation
 * (and therefore without pulling heavy layout libraries into the main bundle).
 */
export interface LayoutEngineDescriptor {
    id: string;
    label: string;
    description: string;
    /** Codicon id rendered in the picker and controls cluster. */
    icon: string;
    /** Tunable parameters this engine exposes as sliders; omitted when it has none. */
    parameters?: readonly LayoutEngineParameter[];
}

// graph/presentation.d.ts
/** Copyright (c) Scott A Dixon */
import React from "react";
export interface GraphIconButtonProps {
    className?: string;
    disabled?: boolean;
    icon: string;
    label: string;
    mode?: "standard" | "micro";
    variant?: "ghost" | "primary" | "danger";
    onClick: React.MouseEventHandler<HTMLButtonElement>;
}
export interface GraphSliderProps {
    disabled?: boolean;
    formatValue?: (value: number) => string;
    label: string;
    max: number;
    min: number;
    onChange: (value: number) => void;
    onCommit?: () => void;
    step: number;
    title?: string;
    value: number;
}
export interface GraphBadgeProps {
    classifier: "controller";
    className?: string;
    helpText?: string;
    icon?: string;
    label: string;
    shape?: "icon" | "pill";
}
/** Presentation slots apply to every graph child, including custom nodes. */
export interface GraphPresentation {
    IconButton: React.ComponentType<GraphIconButtonProps>;
    Slider: React.ComponentType<GraphSliderProps>;
    Badge: React.ComponentType<GraphBadgeProps>;
}
export declare function GraphPresentationProvider({ children, value, }: {
    children: React.ReactNode;
    value?: GraphPresentation;
}): React.JSX.Element;
export declare function GraphIconButton(props: GraphIconButtonProps): React.JSX.Element;
export declare function GraphSlider(props: GraphSliderProps): React.JSX.Element;
export declare function GraphBadge(props: GraphBadgeProps): React.JSX.Element;

// hooks/use-dataflow-canvas-drop.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Manages node drag and drop behaviour for the editor's dataflow canvas.
 */
import React from "react";
import type { DragEndEvent, DragStartEvent } from "@dnd-kit/core";
import type { ReactFlowInstance } from "@xyflow/react";
import type { FlowNodeDefinition as WireFlowNodeDefinition } from "@battersea/flow";
import type { FlowStudioEdge, FlowStudioNode } from "../core/dataflow-editor-state.js";
import type { FlowInteractionPorts } from "../interaction-ports.js";
import { type FlowStudioWorkspaceState } from "../core/flow-persistence.js";
import { type FlowNodeDropFailureReason } from "../core/flow-node-drop-feedback.js";
export type CanvasDropResult = {
    kind: "failure";
    reason: FlowNodeDropFailureReason;
    title: string;
} | {
    kind: "success";
    nextNode: FlowStudioNode;
    title: string;
};
/** Whether the drag has every input required to commit a canvas placement. */
export declare function didDataflowDragResultInPlacement(input: {
    dropZonePresent: boolean;
    overId: string | number | null | undefined;
    pointerPresent: boolean;
    validDragData: boolean;
}): boolean;
/** Distinguishes a deliberate drag from a pointer press released in place. */
export declare function didDataflowPaletteDragMove(input: {
    delta?: {
        x: number;
        y: number;
    } | null;
}): boolean;
/** Builds either the placed node or the user-facing failure classification. */
export declare function resolveCanvasDropResult(options: {
    clientX: number;
    clientY: number;
    dragOffset?: {
        x: number;
        y: number;
    } | null;
    dragPayload: string;
    flowInstance: ReactFlowInstance<FlowStudioNode, FlowStudioEdge> | null;
    nextNodeIndex: number;
    pendingDropTitle: string;
}): CanvasDropResult;
export declare function useDataflowCanvasDrop(options: {
    ports: FlowInteractionPorts;
    commitStructuralChange: (label: string, transform: (workspace: FlowStudioWorkspaceState) => FlowStudioWorkspaceState) => void;
    nodes: readonly FlowStudioNode[];
    setWorkspace: React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>;
}): {
    activeDragDefinition: WireFlowNodeDefinition | null;
    animateRejectedDrop: boolean;
    flowInstanceRef: React.MutableRefObject<ReactFlowInstance<FlowStudioNode, FlowStudioEdge> | null>;
    handlePaletteDragCancel: () => void;
    handlePaletteDragEnd: (event: DragEndEvent) => void;
    handlePaletteDragStart: (event: DragStartEvent) => void;
};

// hooks/use-dataflow-connections.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Manages node and edge mutations for the editor's dataflow canvas.
 */
import React from "react";
import { type Connection, type EdgeChange, type FinalConnectionState, type NodeChange } from "@xyflow/react";
import type { FlowStudioEdge, FlowStudioNode } from "../core/dataflow-editor-state.js";
import type { FlowInteractionPorts } from "../interaction-ports.js";
import type { FlowStudioWorkspaceState } from "../core/flow-persistence.js";
import type { DataflowStructuralChangeOptions } from "./use-dataflow-structural-undo.js";
export interface FlowConnectionAppendResult {
    edges: FlowStudioEdge[];
    status: "duplicate-port" | "incompatible-token" | "invalid" | "updated";
}
export declare function hasStructuralEdgeRemovalChange(changes: readonly EdgeChange<FlowStudioEdge>[]): boolean;
export declare function resolveConnectionAppendResult(options: {
    connection: Connection;
    edges: readonly FlowStudioEdge[];
    nodes?: readonly FlowStudioNode[];
}): FlowConnectionAppendResult;
export declare function useDataflowConnections(options: {
    ports: FlowInteractionPorts;
    commitStructuralChange: (label: string, transform: (workspace: FlowStudioWorkspaceState) => FlowStudioWorkspaceState, options?: DataflowStructuralChangeOptions) => void;
    edges: readonly FlowStudioEdge[];
    nodes: readonly FlowStudioNode[];
    setWorkspace: React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>;
}): {
    handleConnect: (connection: Connection) => void;
    handleConnectEnd: (_: MouseEvent | TouchEvent, connectionState: FinalConnectionState) => void;
    handleEdgesChange: (changes: EdgeChange<FlowStudioEdge>[]) => void;
    handleNodesChange: (changes: NodeChange<FlowStudioNode>[]) => void;
    isConnectionValid: (connection: Connection | FlowStudioEdge) => boolean;
};

// hooks/use-dataflow-flow-lifecycle.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Manages dataflow flow lifecycle state, lifecycle, and derived behaviour for the editor's dataflow workspace.
 */
import React from "react";
import type { FlowNodeDefinition as WireFlowNodeDefinition } from "@battersea/flow";
import type { FlowSummary as WireFlowSummary, FlowDocumentPort, FlowEditorPorts, FlowLifecycleHost, FlowNameDialogState, DataflowDocumentFieldState, FlowNameModalMode } from "../ports.js";
import { type FlowStudioWorkspaceState } from "../core/flow-persistence.js";
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
export declare function useDataflowFlowLifecycle(options: {
    commitStructuralChange: (label: string, transform: (workspace: FlowStudioWorkspaceState) => FlowStudioWorkspaceState, options?: DataflowStructuralChangeOptions) => void;
    host: FlowLifecycleHost;
    documents: FlowDocumentPort;
    validation: FlowEditorPorts["validation"];
    isDirty: boolean;
    nodeDefinitions: readonly WireFlowNodeDefinition[];
    replaceWorkspaceAndResetUndo: (nextWorkspace: FlowStudioWorkspaceState) => void;
    flowSummaries: readonly WireFlowSummary[];
    refreshFlowSummaries: () => Promise<readonly WireFlowSummary[]>;
    setWorkspace: React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>;
    workspace: FlowStudioWorkspaceState;
}): DataflowFlowLifecycleState;

// hooks/use-dataflow-layout.d.ts
import type { LayoutGraph, LayoutPositions, LayoutRunOptions, LayoutEngineDescriptor } from "../graph/layout/types.js";
import type { AuthoringGraphAutoLayoutController } from "../graph.js";
import type { FlowStudioWorkspaceState } from "../core/flow-persistence.js";
import type { DataflowStructuralUndoState } from "./use-dataflow-structural-undo.js";
export interface DataflowLayoutOptions {
    workspace: FlowStudioWorkspaceState;
    commit: DataflowStructuralUndoState["commitLayoutChange"];
    run: (engineId: string, graph: LayoutGraph, options?: LayoutRunOptions) => Promise<LayoutPositions>;
    onError: (error: unknown) => void;
    engines?: readonly LayoutEngineDescriptor[];
}
export declare function useDataflowLayout({ workspace, commit, run, onError, engines, }: DataflowLayoutOptions): AuthoringGraphAutoLayoutController;

// hooks/use-dataflow-selection.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Manages selection, node context menu, and global keyboard behaviour for the editor's dataflow canvas.
 */
import React from "react";
import type { EdgeChange } from "@xyflow/react";
import type { FlowInteractionPorts } from "../interaction-ports.js";
import type { FlowStudioEdge, FlowStudioSelectedEdgeBridge, FlowStudioNode, FlowStudioSelectedEdgeWaypoint, FlowStudioSelectionTarget, NodeContextMenuState } from "../core/dataflow-editor-state.js";
import type { FlowStudioWorkspaceState } from "../core/flow-persistence.js";
import type { FlowPortSide } from "../core/flow-node-ports.js";
import type { FlowPortMoveDirection } from "../core/flow-port-move-controls.js";
export type SelectionKeyboardAction = "clear-selection" | "ignore" | "move-port-toward-end" | "move-port-toward-start" | "remove-bridge-selection" | "remove-node-selection" | "remove-waypoint-selection";
export declare function resolveSelectionKeyboardAction(options: {
    hasNodeContextMenu: boolean;
    isEditableTarget: boolean;
    key: string;
    repeat: boolean;
    selectedEdgeBridge?: FlowStudioSelectedEdgeBridge | null;
    selectedEdgeWaypoint?: FlowStudioSelectedEdgeWaypoint | null;
    selectedTarget?: FlowStudioSelectionTarget;
}): SelectionKeyboardAction;
export declare function shouldDismissNodeContextMenu(target: EventTarget | null): boolean;
export declare function shouldClearSelectionForEdgeChanges(changes: readonly EdgeChange<FlowStudioEdge>[]): boolean;
export declare function selectFlowInWorkspace(workspace: FlowStudioWorkspaceState): FlowStudioWorkspaceState;
export declare function useDataflowSelection(options: {
    ports: FlowInteractionPorts;
    clearSelectedEdgeBridge: () => void;
    clearSelectedEdgeWaypoint: () => void;
    moveSelectedPort: (direction: FlowPortMoveDirection) => void;
    removeNode: (nodeId: string) => void;
    removeSelectedEdgeBridge: () => void;
    removeSelectedEdgeWaypoint: () => void;
    selectedEdgeBridge: FlowStudioSelectedEdgeBridge | null;
    selectedEdgeWaypoint: FlowStudioSelectedEdgeWaypoint | null;
    selectedTarget: FlowStudioSelectionTarget;
    setWorkspace: React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>;
}): {
    clearSelection: () => void;
    handleDetailPaneClose: () => void;
    handleNodeClick: (_: React.MouseEvent, node: FlowStudioNode) => void;
    handleNodeContextMenu: (event: React.MouseEvent, node: FlowStudioNode) => void;
    handlePortSelect: (optionsForPort: {
        nodeId: string;
        portId: string;
        side: FlowPortSide;
    }) => void;
    nodeContextMenu: NodeContextMenuState | null;
    selectFlow: () => void;
    setNodeContextMenu: React.Dispatch<React.SetStateAction<NodeContextMenuState | null>>;
};

// hooks/use-dataflow-structural-undo.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Owns structural dataflow undo history and restore behaviour. Backed
 * directly by `react-amnesia` — keyboard routing comes from the editor
 * shell's multi-scope provider.
 */
import React from "react";
import { type Amnesia } from "react-amnesia";
import { type DataflowStructuralSnapshot } from "../core/dataflow-undo.js";
import type { FlowStudioWorkspaceState } from "../core/flow-persistence.js";
export declare const DATAFLOW_STRUCTURAL_UNDO_SCOPE_ID = "dataflow:structural";
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
    commitLayoutChange: (label: string, transform: (workspace: FlowStudioWorkspaceState) => FlowStudioWorkspaceState, options?: DataflowStructuralChangeOptions) => void;
    commitStructuralChange: (label: string, transform: (workspace: FlowStudioWorkspaceState) => FlowStudioWorkspaceState, options?: DataflowStructuralChangeOptions) => void;
    pushPresentLayoutChange: (label: string, previousSnapshot: DataflowStructuralSnapshot, options?: DataflowStructuralChangeOptions) => void;
    setTrackedWorkspace: React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>;
    replaceWorkspaceAndResetUndo: (nextWorkspace: FlowStudioWorkspaceState) => void;
    undoSelectionResetToken: number;
}
export declare function useDataflowStructuralUndo(options: {
    setWorkspace: React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>;
    workspace: FlowStudioWorkspaceState;
}): DataflowStructuralUndoState;

// hooks/use-flow-canvas-editing.d.ts
/**
 * Copyright (c) Scott A Dixon
 *
 * Composes focused dataflow canvas hooks for the editor's dataflow workspace.
 */
import React from "react";
import type { EdgeChange } from "@xyflow/react";
import type { FlowStudioEdge, FlowStudioNode } from "../core/dataflow-editor-state.js";
import { type FlowPortMoveDirection } from "../core/flow-port-move-controls.js";
import { type FlowControllerPortPlacement } from "../core/flow-controller-port-placement.js";
import { type FlowPortSide, type FlowStudioResolvedPort } from "../core/flow-node-ports.js";
import type { FlowNodeDefinition } from "@battersea/flow";
import type { FlowInteractionPorts } from "../interaction-ports.js";
import type { DataflowStructuralUndoState } from "./use-dataflow-structural-undo.js";
import type { FlowStudioWorkspaceState } from "../core/flow-persistence.js";
export interface FlowCanvasEditingOptions extends Pick<DataflowStructuralUndoState, "captureSnapshot" | "commitLayoutChange" | "commitStructuralChange" | "pushPresentLayoutChange" | "undoSelectionResetToken"> {
    ports: FlowInteractionPorts;
    workspace: FlowStudioWorkspaceState;
    setWorkspace: React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>;
    nodeDefinitionLookup: Record<string, FlowNodeDefinition>;
    selectedNode: FlowStudioNode | null;
    selectedNodeDefinition: FlowNodeDefinition | null;
    selectedPort: FlowStudioResolvedPort | null;
}
export declare function useFlowCanvasEditing(options: FlowCanvasEditingOptions): {
    activeDragDefinition: FlowNodeDefinition | null;
    animateRejectedDrop: boolean;
    clearSelection: () => void;
    flowInstanceRef: React.MutableRefObject<import("@xyflow/react").ReactFlowInstance<FlowStudioNode, FlowStudioEdge> | null>;
    handleConnect: (connection: import("@xyflow/react").Connection) => void;
    handleConnectEnd: (_: MouseEvent | TouchEvent, connectionState: import("@xyflow/react").FinalConnectionState) => void;
    handleDetailPaneClose: () => void;
    edgeGestureDriver: import("../graph.js").AuthoringGraphInteractiveGestureDriver;
    handleEdgesChange: (changes: EdgeChange<FlowStudioEdge>[]) => void;
    handleInsertEdgeBridge: (edgeKey: string, segmentIndex: number, bridge: import("../graph.js").AuthoringGraphBridge) => void;
    handleInsertEdgeWaypoint: (edgeKey: string, segmentIndex: number, segmentT: number, waypoint: import("../graph.js").AuthoringGraphWaypoint) => void;
    handleNodeClick: (_: React.MouseEvent, node: FlowStudioNode) => void;
    handleNodeContextMenu: (event: React.MouseEvent, node: FlowStudioNode) => void;
    handleNodeDragStart: () => void;
    handleNodeDragStop: () => void;
    handleNodesChange: (changes: import("@xyflow/react").NodeChange<FlowStudioNode>[]) => void;
    handlePaneClick: () => void;
    handlePortSelect: (optionsForPort: {
        nodeId: string;
        portId: string;
        side: FlowPortSide;
    }) => void;
    handlePaletteDragCancel: () => void;
    handlePaletteDragEnd: (event: import("@dnd-kit/core").DragEndEvent) => void;
    handlePaletteDragStart: (event: import("@dnd-kit/core").DragStartEvent) => void;
    handleLogicVisualDirectionChange: (placement: FlowControllerPortPlacement) => void;
    handleMovePort: (target: {
        direction: FlowPortMoveDirection;
        nodeId: string;
        portId: string;
        side: FlowPortSide;
    }) => void;
    handleSelectEdgeBridge: (edgeKey: string, bridgeIndex: number) => void;
    handleSelectEdgeWaypoint: (edgeKey: string, waypointIndex: number) => void;
    handleUpdateEdgeBridgeGap: (edgeKey: string, bridgeIndex: number, gap: number) => void;
    handleUpdateEdgeBridgePosition: (edgeKey: string, bridgeIndex: number, segmentIndex: number, t: number) => void;
    handleUpdateEdgeWaypointHandle: (edgeKey: string, waypointIndex: number, handleKind: "inHandle" | "outHandle", independent: boolean, position: import("../graph.js").AuthoringGraphPoint) => void;
    handleUpdateEdgeWaypointPosition: (edgeKey: string, waypointIndex: number, position: import("../graph.js").AuthoringGraphPoint) => void;
    isConnectionValid: (connection: import("@xyflow/react").Connection | FlowStudioEdge) => boolean;
    movedPortPulse: {
        nodeId: string;
        portId: string;
        replay: "a" | "b";
        side: FlowPortSide;
    } | null;
    nodeContextMenu: import("../core/dataflow-editor-state.js").NodeContextMenuState | null;
    removeNode: (nodeId: string) => void;
    selectedEdgeBridge: {
        bridgeIndex: number;
        edgeId: string;
    } | null;
    selectedEdgeWaypoint: {
        edgeId: string;
        waypointIndex: number;
    } | null;
    selectedPort: FlowStudioResolvedPort | null;
};

// hooks/use-flow-edge-activity.d.ts
import type { FlowStudioEdge } from "../core/dataflow-editor-state.js";
import { type FlowStudioEdgeActivationState, type FlowActivityRecord } from "../core/flow-edge-activation.js";
export declare function useFlowEdgeActivity(options: {
    edges: readonly FlowStudioEdge[];
    subscribe: ((listener: (record: FlowActivityRecord) => void) => () => void) | null;
}): FlowStudioEdgeActivationState;

// index.d.ts
export * from "./graph.js";
export * from "./components/dataflow-canvas.js";
export * from "./core/flow-canvas-node.js";
export * from "./hooks/use-dataflow-structural-undo.js";
export * from "./interaction-ports.js";
export * from "./hooks/use-flow-canvas-editing.js";
export * from "./ports.js";
export * from "./hooks/use-dataflow-flow-lifecycle.js";
export * from "./components/dataflow-node-palette.js";
export * from "./components/flow-parameter-control.js";
export * from "./components/flow-inspector.js";
export * from "./hooks/use-dataflow-layout.js";
export * from "./components/flow-editor.js";
export * from "./hooks/use-flow-edge-activity.js";

// interaction-ports.d.ts
/** Copyright (c) Scott A Dixon */
import type { DragEndEvent } from "@dnd-kit/core";
export interface FlowNotification {
    id?: string;
    message: string;
    tone: "ready" | "warning" | "error";
    title?: string;
}
export interface FlowInteractionPorts {
    notify: (notification: FlowNotification) => void;
    isKeyboardBlocked: (event: KeyboardEvent) => boolean;
    resolveDragGrabOffset: (event: DragEndEvent) => {
        x: number;
        y: number;
    } | null;
    resolveDragReleasePoint: (event: DragEndEvent) => {
        x: number;
        y: number;
    } | null;
}

// ports.d.ts
/** Copyright (c) Scott A Dixon */
import type { FlowDocument, FlowNodeDefinition, FlowValidationResult } from "@battersea/flow";
import type { FlowNotification } from "./interaction-ports.js";
export interface FlowSummary {
    flow_key: string;
    title: string;
    unavailable_reason?: string;
}
export interface FlowDocumentPort {
    read: (key: string) => Promise<FlowDocument>;
    list: () => Promise<readonly FlowSummary[]>;
    save: (document: FlowDocument) => Promise<{
        flow: FlowDocument;
    }>;
    clone: (request: {
        current_flow_key: string;
        next_flow_key: string;
        next_title: string;
    }) => Promise<{
        flow: FlowDocument;
    }>;
    delete: (request: {
        flow_key: string;
    }) => Promise<unknown>;
}
export interface FlowConfirmOptions {
    cancelLabel?: string;
    actionLabel: string;
    message: string;
    title: string;
}
export interface FlowDirtyRegistration {
    canSave: boolean;
    isDirty: boolean;
    save: () => Promise<void>;
    confirmDiscard: (options: FlowConfirmOptions) => Promise<boolean>;
    saveLabel: string;
    saveTitle: string;
}
export interface FlowLifecycleHost {
    confirm: (options: FlowConfirmOptions) => Promise<boolean>;
    confirmDiscard: (options: FlowConfirmOptions) => Promise<boolean>;
    registerDirty: (registration: FlowDirtyRegistration) => void;
    unregisterDirty: () => void;
    notify: (notification: FlowNotification) => void;
    deleteConfirmation: (title: string, dirty: boolean) => string;
}
export type FlowActivationCommand<TResult = FlowActivation> = (request: FlowActivationRequest) => Promise<TResult>;
export interface FlowActivationRequest {
    flow_key: string;
    node_id: string;
    parameters: Record<string, unknown>;
}
export interface FlowActivation {
    id: string;
    status: "running" | "succeeded" | "failed" | "cancelled";
}
export interface FlowDiagnostic {
    activation_id: string;
    node_id?: string;
    sequence: number;
    phase: string;
    detail?: unknown;
}
/** The embedding application supplies I/O, execution admission and subscription lifetimes. */
export interface FlowEditorPorts {
    documents: FlowDocumentPort;
    catalogue: {
        read: () => Promise<readonly FlowNodeDefinition[]>;
    };
    validation: {
        validate: (document: FlowDocument) => Promise<FlowValidationResult>;
    };
    activation: {
        activate: FlowActivationCommand;
        cancel: (id: string) => Promise<void>;
        subscribe: (id: string, listener: (event: FlowDiagnostic) => void) => () => void;
    };
}
export interface DataflowDocumentFieldState {
    message: string;
    status: "idle" | "loading" | "error";
}
export type FlowNameModalMode = "clone" | "name" | "rename" | "rename-node";
export interface FlowNameDialogState {
    initialValue: string;
    isOpen: boolean;
    mode: FlowNameModalMode;
    pendingAction: "clone" | "save" | "rename" | "rename-node" | null;
    targetNodeId: string | null;
    targetFlowKey: string | null;
}
