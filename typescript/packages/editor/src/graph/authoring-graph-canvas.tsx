/**
 * Copyright (c) Scott A Dixon
 *
 * Hosts the shared React Flow substrate used by editor authoring canvases.
 */
import React from "react";
import {
  GraphPresentationProvider,
  type GraphPresentation,
} from "./presentation.js";
import { useDroppable } from "@dnd-kit/core";
import {
  Controls,
  ReactFlow,
  type Connection,
  type ControlProps,
  type DefaultEdgeOptions,
  type Edge,
  type EdgeChange,
  type EdgeMouseHandler,
  type EdgeTypes,
  type FinalConnectionState,
  type Node,
  type NodeChange,
  type OnMove,
  type OnNodeDrag,
  type NodeMouseHandler,
  type NodeTypes,
  type OnReconnect,
  type ReactFlowInstance,
} from "@xyflow/react";

import {
  AuthoringGraphAutoLayoutControl,
  type AuthoringGraphAutoLayoutController,
} from "./authoring-graph-auto-layout-control.js";

export type {
  AuthoringGraphAutoLayoutController,
  AuthoringGraphAutoLayoutStatus,
} from "./authoring-graph-auto-layout-control.js";

export interface AuthoringGraphCanvasReactFlowProps<
  TNode extends Node = Node,
  TEdge extends Edge = Edge,
> {
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

export interface AuthoringGraphCanvasProps<
  TNode extends Node = Node,
  TEdge extends Edge = Edge,
> {
  presentation?: GraphPresentation;
  canvasClassName: string;
  autoLayoutController?: AuthoringGraphAutoLayoutController;
  controlsProps?: false | ControlProps;
  dropzoneId: string;
  edgeTypes: EdgeTypes;
  edges: readonly TEdge[];
  flowInstanceRef: React.MutableRefObject<ReactFlowInstance<
    TNode,
    TEdge
  > | null>;
  isValidConnection?: (connection: Connection | TEdge) => boolean;
  nodeTypes: NodeTypes;
  nodes: readonly TNode[];
  onConnect?: (connection: Connection) => void;
  onConnectEnd?: (
    event: MouseEvent | TouchEvent,
    connectionState: FinalConnectionState,
  ) => void;
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
  paneRef?:
    | React.RefCallback<HTMLDivElement>
    | React.MutableRefObject<HTMLDivElement | null>
    | null;
  reactFlowProps?: AuthoringGraphCanvasReactFlowProps<TNode, TEdge>;
}

export function AuthoringGraphCanvas<
  TNode extends Node = Node,
  TEdge extends Edge = Edge,
>({
  presentation,
  canvasClassName,
  autoLayoutController,
  controlsProps = false,
  dropzoneId,
  edgeTypes,
  edges,
  flowInstanceRef,
  isValidConnection,
  nodeTypes,
  nodes,
  onConnect,
  onConnectEnd,
  onEdgesChange,
  onFlowInit,
  onNodeDrag,
  onNodeDragStart,
  onNodeDragStop,
  onNodeClick,
  onNodeContextMenu,
  onNodesChange,
  onPaneClick,
  onViewportSizeChange,
  overlay,
  paneClassName,
  paneRef: externalPaneRef,
  reactFlowProps,
}: AuthoringGraphCanvasProps<TNode, TEdge>): React.JSX.Element {
  const { setNodeRef } = useDroppable({
    id: dropzoneId,
  });
  const paneRef = React.useRef<HTMLDivElement | null>(null);
  const resolvedPaneClassName = ["authoring-graph-canvas-pane", paneClassName]
    .filter(Boolean)
    .join(" ");
  const resolvedCanvasClassName = ["authoring-graph-canvas", canvasClassName]
    .filter(Boolean)
    .join(" ");
  const handlePaneRef = React.useCallback(
    (node: HTMLDivElement | null) => {
      paneRef.current = node;
      setNodeRef(node);
      if (typeof externalPaneRef === "function") {
        externalPaneRef(node);
      } else if (externalPaneRef) {
        externalPaneRef.current = node;
      }
    },
    [externalPaneRef, setNodeRef],
  );

  React.useEffect(() => {
    if (
      !onViewportSizeChange ||
      !paneRef.current ||
      typeof ResizeObserver === "undefined"
    ) {
      return undefined;
    }

    const node = paneRef.current;
    const notifySize = () => {
      onViewportSizeChange({
        height: node.clientHeight,
        width: node.clientWidth,
      });
    };
    const resizeObserver = new ResizeObserver(notifySize);
    resizeObserver.observe(node);
    notifySize();
    return () => {
      resizeObserver.disconnect();
    };
  }, [onViewportSizeChange]);

  // React Flow wants mutable arrays; copy ONLY when the (memoized) source
  // arrays actually change, instead of allocating a fresh array every render
  // and forcing React Flow to re-diff the whole graph on each parent render.
  const mutableNodes = React.useMemo(() => [...nodes], [nodes]);
  const mutableEdges = React.useMemo(() => [...edges], [edges]);

  return (
    <GraphPresentationProvider value={presentation}>
      <div className={resolvedPaneClassName} ref={handlePaneRef}>
        <ReactFlow<TNode, TEdge>
          aria-label={reactFlowProps?.ariaLabel}
          className={resolvedCanvasClassName}
          connectionLineStyle={reactFlowProps?.connectionLineStyle}
          defaultEdgeOptions={reactFlowProps?.defaultEdgeOptions}
          edgeTypes={edgeTypes}
          edges={mutableEdges}
          edgesReconnectable={reactFlowProps?.edgesReconnectable}
          fitView={reactFlowProps?.fitView}
          isValidConnection={isValidConnection}
          maxZoom={reactFlowProps?.maxZoom}
          minZoom={reactFlowProps?.minZoom}
          nodeTypes={nodeTypes}
          nodes={mutableNodes}
          nodesDraggable={reactFlowProps?.nodesDraggable}
          onlyRenderVisibleElements={
            reactFlowProps?.onlyRenderVisibleElements ?? false
          }
          onConnect={onConnect}
          onConnectEnd={onConnectEnd}
          onEdgeClick={reactFlowProps?.onEdgeClick}
          onEdgesChange={onEdgesChange}
          onMove={reactFlowProps?.onMove}
          onInit={(instance) => {
            flowInstanceRef.current = instance;
            onFlowInit?.(instance);
          }}
          onNodeDrag={onNodeDrag}
          onNodeDragStart={onNodeDragStart}
          onNodeDragStop={onNodeDragStop}
          onNodeClick={onNodeClick}
          onNodeContextMenu={onNodeContextMenu}
          onNodesChange={onNodesChange}
          onPaneClick={onPaneClick}
          onReconnect={reactFlowProps?.onReconnect}
          proOptions={{ hideAttribution: true }}
        >
          {controlsProps !== false ? (
            <Controls {...controlsProps}>
              {controlsProps.children}
              <AuthoringGraphAutoLayoutControl
                controller={autoLayoutController}
                paneRef={paneRef}
              />
            </Controls>
          ) : null}
        </ReactFlow>
        {overlay}
      </div>
    </GraphPresentationProvider>
  );
}
