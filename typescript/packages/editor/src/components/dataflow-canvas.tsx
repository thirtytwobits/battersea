/**
 * Copyright (c) Scott A Dixon
 *
 * Renders the dataflow canvas component for the editor's dataflow workspace.
 */
import React from "react";
import type { GraphPresentation } from "../graph.js";
import {
  type Connection,
  type EdgeChange,
  type EdgeTypes,
  type FinalConnectionState,
  type NodeChange,
  type NodeTypes,
  type OnNodeDrag,
  type ReactFlowInstance,
} from "@xyflow/react";
import type { AuthoringGraphInteractiveGestureDriver } from "../graph.js";
import type { AuthoringGraphCanvasViewportSize } from "../graph.js";

import { AuthoringGraphCanvas } from "../graph.js";
import { AuthoringEditableEdgeView } from "../graph.js";
import {
  FlowStudioCanvasNode,
  type FlowStudioCanvasNodeData,
} from "../core/flow-canvas-node.js";
import type {
  FlowStudioEdge,
  FlowStudioEdgeData,
  FlowStudioEdgeBridge,
  FlowStudioNode,
  FlowStudioPoint,
  FlowStudioPortSelectionTarget,
  FlowStudioSelectedEdgeBridge,
  FlowStudioSelectedEdgeWaypoint,
  FlowStudioEdgeWaypoint,
  NodeContextMenuState,
} from "../core/dataflow-editor-state.js";
import {
  isArrayTokenType,
  resolveFlowCanvasEdge,
} from "../core/flow-canvas-edges.js";
import type { FlowStudioEdgeActivationState } from "../core/flow-edge-activation.js";
import type { FlowPortSide } from "../core/flow-node-ports.js";
import { DATAFLOW_CANVAS_DROPZONE_ID } from "../core/flow-drag.js";

const DATAFLOW_MIN_ZOOM = 0.03;
const DATAFLOW_CANVAS_CONTROLS_PROPS = {
  fitViewOptions: {
    duration: 180,
    padding: 0.18,
    minZoom: DATAFLOW_MIN_ZOOM,
  },
  showZoom: false,
} as const;
const DATAFLOW_INITIAL_FIT_VIEW_OPTIONS = {
  minZoom: DATAFLOW_MIN_ZOOM,
  duration: 0,
  padding: 0.18,
} as const;

const FLOW_STUDIO_EDGE_THEME = {
  addButtonShellClassName: "flow-studio-edge__add-button-shell",
  bridgeButtonClassName: "flow-studio-edge__bridge-button",
  bridgeButtonGlyphClassName: "flow-studio-edge__bridge-button-glyph",
  bridgeCapClassName: "flow-studio-edge__bridge-cap",
  bridgeDragHandleClassName: "flow-studio-edge-bridge__drag-handle",
  bridgeGapHandleClassName: "flow-studio-edge-bridge__gap-handle",
  bridgeGapLineClassName: "flow-studio-edge-bridge__gap-line",
  bridgeGroupClassName: "flow-studio-edge-bridge",
  bridgeHitTargetClassName: "flow-studio-edge__bridge-hit-target",
  bridgeSelectedClassName: "flow-studio-edge-bridge--selected",
  interactionClassName: "flow-studio-edge__interaction",
  secondaryPathClassName: "flow-studio-edge__fragment-array-gap",
  selectionOutlineClassName: "flow-studio-edge__selection-outline",
  waypointAnchorClassName: "flow-studio-edge-waypoint__anchor",
  waypointGroupClassName: "flow-studio-edge-waypoint",
  waypointHandleClassName: "flow-studio-edge-waypoint__handle",
  waypointHandleInClassName: "flow-studio-edge-waypoint__handle--in",
  waypointHandleLineClassName: "flow-studio-edge-waypoint__handle-line",
  waypointHandleOutClassName: "flow-studio-edge-waypoint__handle--out",
  waypointSelectedClassName: "flow-studio-edge-waypoint--selected",
} as const;

const DATAFLOW_CANVAS_EDGE_TYPES: EdgeTypes = {
  flowStudio: AuthoringEditableEdgeView,
};

const DATAFLOW_CANVAS_NODE_TYPES: NodeTypes = {
  flowStudio: FlowStudioCanvasNode,
};

const EMPTY_VIEWPORT_SIZE: AuthoringGraphCanvasViewportSize = {
  height: 0,
  width: 0,
};

export interface DataflowCanvasProps {
  presentation?: GraphPresentation;
  autoLayout?: import("../graph.js").AuthoringGraphAutoLayoutController;
  autoFitViewKey?: string;
  edgeActivationState: FlowStudioEdgeActivationState;
  edges: readonly FlowStudioEdge[];
  flowInstanceRef: React.MutableRefObject<ReactFlowInstance<
    FlowStudioNode,
    FlowStudioEdge
  > | null>;
  edgeGestureDriver: AuthoringGraphInteractiveGestureDriver;
  isConnectionValid: (connection: Connection | FlowStudioEdge) => boolean;
  nodeContextMenu: NodeContextMenuState | null;
  nodes: readonly FlowStudioNode[];
  onConnect: (connection: Connection) => void;
  onConnectEnd: (
    event: MouseEvent | TouchEvent,
    connectionState: FinalConnectionState,
  ) => void;
  onEdgesChange: (changes: EdgeChange<FlowStudioEdge>[]) => void;
  onNodeClick: (_event: React.MouseEvent, node: FlowStudioNode) => void;
  onNodeContextMenu: (event: React.MouseEvent, node: FlowStudioNode) => void;
  onNodesChange: (changes: NodeChange<FlowStudioNode>[]) => void;
  onPaneClick: () => void;
  onInsertEdgeBridge: (
    edgeId: string,
    segmentIndex: number,
    bridge: FlowStudioEdgeBridge,
  ) => void;
  onInsertEdgeWaypoint: (
    edgeId: string,
    segmentIndex: number,
    segmentT: number,
    waypoint: FlowStudioEdgeWaypoint,
  ) => void;
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
  onUpdateEdgeBridgeGap: (
    edgeId: string,
    bridgeIndex: number,
    gap: number,
  ) => void;
  onUpdateEdgeBridgePosition: (
    edgeId: string,
    bridgeIndex: number,
    segmentIndex: number,
    t: number,
  ) => void;
  onUpdateEdgeWaypointHandle: (
    edgeId: string,
    waypointIndex: number,
    handleKind: "inHandle" | "outHandle",
    independent: boolean,
    position: FlowStudioPoint,
  ) => void;
  onUpdateEdgeWaypointPosition: (
    edgeId: string,
    waypointIndex: number,
    position: FlowStudioPoint,
  ) => void;
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

export function DataflowCanvas({
  autoLayout,
  presentation,
  autoFitViewKey,
  edgeActivationState,
  edgeGestureDriver,
  edges,
  flowInstanceRef,
  isConnectionValid,
  nodeContextMenu,
  nodes,
  onConnect,
  onConnectEnd,
  onEdgesChange,
  onNodeClick,
  onNodeContextMenu,
  onNodesChange,
  onPaneClick,
  onInsertEdgeBridge,
  onInsertEdgeWaypoint,
  onMovePort,
  onPortSelect,
  onNodeDragStart,
  onNodeDragStop,
  onRemoveNode,
  onSelectEdgeBridge,
  onSelectEdgeWaypoint,
  onUpdateEdgeBridgeGap,
  onUpdateEdgeBridgePosition,
  onUpdateEdgeWaypointHandle,
  onUpdateEdgeWaypointPosition,
  movedPortPulse,
  selectedEdgeBridge,
  selectedEdgeWaypoint,
  selectedPort,
}: DataflowCanvasProps): React.JSX.Element {
  const fittedViewKeyRef = React.useRef<string | null>(null);
  const [fittedViewKey, setFittedViewKey] = React.useState<string | null>(null);
  const [flowInstanceReadyToken, setFlowInstanceReadyToken] = React.useState(0);
  const [viewportSize, setViewportSize] =
    React.useState<AuthoringGraphCanvasViewportSize>(EMPTY_VIEWPORT_SIZE);
  const resolvedEdges = React.useMemo(
    () =>
      edges.map((edge) => {
        const resolvedEdge = resolveFlowCanvasEdge(
          edge,
          nodes,
          edgeActivationState[edge.id] ?? null,
        );
        const data: FlowStudioEdgeData = {
          addBridgeLabel: "Add bridge",
          addWaypointLabel: "Add waypoint",
          bridges: resolvedEdge.data?.bridges,
          edgeClassName: resolvedEdge.className,
          gestureDriver: edgeGestureDriver,
          kind: resolvedEdge.data?.kind === "signal" ? "signal" : "token",
          onInsertBridge: (
            segmentIndex: number,
            bridge: FlowStudioEdgeBridge,
          ) => {
            onInsertEdgeBridge(edge.id, segmentIndex, bridge);
          },
          onSelectBridge: (bridgeIndex: number) => {
            onSelectEdgeBridge(edge.id, bridgeIndex);
          },
          onUpdateBridgeGap: (bridgeIndex: number, gap: number) => {
            onUpdateEdgeBridgeGap(edge.id, bridgeIndex, gap);
          },
          onUpdateBridgePosition: (
            bridgeIndex: number,
            segmentIndex: number,
            t: number,
          ) => {
            onUpdateEdgeBridgePosition(edge.id, bridgeIndex, segmentIndex, t);
          },
          order:
            typeof resolvedEdge.data?.order === "number"
              ? resolvedEdge.data.order
              : 0,
          selectedBridgeIndex:
            selectedEdgeBridge?.edgeId === edge.id
              ? selectedEdgeBridge.bridgeIndex
              : undefined,
          showSecondaryPath: isArrayTokenType(resolvedEdge.data?.tokenType),
          theme: {
            ...FLOW_STUDIO_EDGE_THEME,
            pathClassName: resolvedEdge.className,
          },
          tokenType: resolvedEdge.data?.tokenType,
          waypoints: resolvedEdge.data?.waypoints,
          selectedWaypointIndex:
            selectedEdgeWaypoint?.edgeId === edge.id
              ? selectedEdgeWaypoint.waypointIndex
              : undefined,
          onInsertWaypoint: (
            segmentIndex: number,
            segmentT: number,
            waypoint: FlowStudioEdgeWaypoint,
          ) => {
            onInsertEdgeWaypoint(edge.id, segmentIndex, segmentT, waypoint);
          },
          onSelectWaypoint: (waypointIndex: number) => {
            onSelectEdgeWaypoint(edge.id, waypointIndex);
          },
          onUpdateWaypointHandle: (
            waypointIndex: number,
            handleKind: "inHandle" | "outHandle",
            independent: boolean,
            position: FlowStudioPoint,
          ) => {
            onUpdateEdgeWaypointHandle(
              edge.id,
              waypointIndex,
              handleKind,
              independent,
              position,
            );
          },
          onUpdateWaypointPosition: (
            waypointIndex: number,
            position: FlowStudioPoint,
          ) => {
            onUpdateEdgeWaypointPosition(edge.id, waypointIndex, position);
          },
        };

        return {
          ...resolvedEdge,
          data,
        };
      }),
    [
      edgeActivationState,
      edges,
      nodes,
      edgeGestureDriver,
      onInsertEdgeBridge,
      onInsertEdgeWaypoint,
      onSelectEdgeBridge,
      onSelectEdgeWaypoint,
      onUpdateEdgeBridgeGap,
      onUpdateEdgeBridgePosition,
      onUpdateEdgeWaypointHandle,
      onUpdateEdgeWaypointPosition,
      selectedEdgeBridge,
      selectedEdgeWaypoint,
    ],
  );
  const resolvedNodes = React.useMemo(
    () =>
      nodes.map((node) => ({
        ...node,
        data: {
          ...node.data,
          canvasMovedPortPulse: movedPortPulse ?? null,
          canvasOnMovePort: onMovePort,
          canvasOnPortSelect: onPortSelect,
          canvasSelectedPort: selectedPort ?? null,
        } satisfies FlowStudioCanvasNodeData,
      })),
    [movedPortPulse, nodes, onMovePort, onPortSelect, selectedPort],
  );
  React.useEffect(() => {
    const nextFitKey = autoFitViewKey?.trim() ?? "";
    const instance = flowInstanceRef.current;
    const viewportReady = viewportSize.width > 0 && viewportSize.height > 0;
    if (!nextFitKey) {
      if (fittedViewKeyRef.current !== null) {
        fittedViewKeyRef.current = null;
        setFittedViewKey(null);
      }
      return undefined;
    }

    if (
      typeof window === "undefined" ||
      !instance ||
      resolvedNodes.length === 0 ||
      !viewportReady ||
      fittedViewKeyRef.current === nextFitKey
    ) {
      return undefined;
    }

    let cancelled = false;
    const scheduleFitView =
      typeof window.requestAnimationFrame === "function"
        ? (callback: FrameRequestCallback) => {
            let secondFrameId: number | null = null;
            const firstFrameId = window.requestAnimationFrame((timestamp) => {
              secondFrameId = window.requestAnimationFrame(() =>
                callback(timestamp),
              );
            });
            return () => {
              window.cancelAnimationFrame(firstFrameId);
              if (secondFrameId !== null) {
                window.cancelAnimationFrame(secondFrameId);
              }
            };
          }
        : (callback: FrameRequestCallback) => {
            const id = window.setTimeout(() => callback(Date.now()), 0);
            return () => window.clearTimeout(id);
          };
    const cancelScheduledFitView = scheduleFitView(() => {
      if (cancelled || fittedViewKeyRef.current === nextFitKey) {
        return;
      }

      void instance
        .fitView(DATAFLOW_INITIAL_FIT_VIEW_OPTIONS)
        .then((fitted) => {
          if (!cancelled && fitted) {
            fittedViewKeyRef.current = nextFitKey;
            setFittedViewKey(nextFitKey);
          }
        });
    });

    return () => {
      cancelled = true;
      cancelScheduledFitView();
    };
  }, [
    autoFitViewKey,
    flowInstanceReadyToken,
    flowInstanceRef,
    resolvedNodes.length,
    viewportSize.height,
    viewportSize.width,
  ]);

  const resolvedAutoFitViewKey = autoFitViewKey?.trim() ?? "";
  const shouldHideUntilInitialFit =
    Boolean(resolvedAutoFitViewKey) &&
    resolvedNodes.length > 0 &&
    fittedViewKey !== resolvedAutoFitViewKey;
  const canvasClassName = shouldHideUntilInitialFit
    ? "flow-studio-canvas flow-studio-canvas--pending-initial-fit"
    : "flow-studio-canvas";

  return (
    <AuthoringGraphCanvas<FlowStudioNode, FlowStudioEdge>
      reactFlowProps={{ minZoom: DATAFLOW_MIN_ZOOM }}
      autoLayoutController={autoLayout}
      presentation={presentation}
      canvasClassName={canvasClassName}
      controlsProps={DATAFLOW_CANVAS_CONTROLS_PROPS}
      dropzoneId={DATAFLOW_CANVAS_DROPZONE_ID}
      edgeTypes={DATAFLOW_CANVAS_EDGE_TYPES}
      edges={resolvedEdges}
      flowInstanceRef={flowInstanceRef}
      isValidConnection={isConnectionValid}
      nodeTypes={DATAFLOW_CANVAS_NODE_TYPES}
      nodes={resolvedNodes}
      onFlowInit={() => setFlowInstanceReadyToken((current) => current + 1)}
      onViewportSizeChange={setViewportSize}
      onConnect={onConnect}
      onConnectEnd={onConnectEnd}
      onEdgesChange={onEdgesChange}
      onNodeClick={onNodeClick}
      onNodeContextMenu={onNodeContextMenu}
      onNodeDragStart={onNodeDragStart}
      onNodeDragStop={onNodeDragStop}
      onNodesChange={onNodesChange}
      onPaneClick={onPaneClick}
      overlay={
        nodeContextMenu ? (
          <div
            className="flow-studio-node-context-menu"
            onPointerDown={(event) => event.stopPropagation()}
            style={{
              left: `${nodeContextMenu.x}px`,
              top: `${nodeContextMenu.y}px`,
            }}
          >
            <button
              className="flow-studio-node-context-menu__action"
              onClick={() => onRemoveNode(nodeContextMenu.nodeId)}
              type="button"
            >
              Remove
            </button>
          </div>
        ) : null
      }
      paneClassName="flow-studio-flow-pane"
    />
  );
}
