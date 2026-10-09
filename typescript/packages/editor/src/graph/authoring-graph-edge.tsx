/**
 * Copyright (c) Scott A Dixon
 *
 * Renders the shared interactive edge layer used by authoring-graph canvases.
 */
import React from "react";
import {
  BaseEdge,
  EdgeLabelRenderer,
  getBezierPath,
  useReactFlow,
  useStore,
  type Edge,
  type EdgeProps,
} from "@xyflow/react";
import { GraphIconButton as IconButton } from "./presentation.js";

import {
  buildEdgeInsertionIntervals,
  buildVisibleEdgePathWithBridges,
  createBridgeForInterval,
  resolveDirectionMarkerGeometry,
  resolveBridgeGapFromHandle,
  resolveBridgeGeometry,
  resolveInsertionIntervalMidpoint,
  resolveNearestBridgeLocation,
  resolveNearestInsertionInterval,
  resolveTargetBoundaryDirectionMarkerGeometry,
  type AuthoringGraphDirectionMarkerBoundary,
} from "./authoring-graph-edge-bridges.js";
import type {
  AuthoringGraphBridge,
  AuthoringGraphEdgeLayout,
  AuthoringGraphEdgeSelection,
  AuthoringGraphPoint,
  AuthoringGraphWaypoint,
} from "./authoring-graph-edge-types.js";
import {
  type AuthoringGraphInteractiveGestureSession,
  createAuthoringGraphInteractiveGestureSession,
  resolveAuthoringGraphEdgeGestureLabel,
  type AuthoringGraphInteractiveGestureDriver,
} from "./authoring-graph-gestures.js";
import {
  buildAuthoringGraphWaypointPath,
  buildAuthoringGraphWaypointSegments,
  createAuthoringGraphWaypointForSegmentAt,
} from "./authoring-graph-edge-waypoints.js";

interface EdgeDragState {
  bridgeIndex?: number;
  handleKind: "anchor" | "bridge" | "bridgeGap" | "inHandle" | "outHandle";
  waypointIndex?: number;
}

interface ActiveAuthoringGraphEdgeDragSession {
  bridgeGeometries: ReturnType<typeof resolveBridgeGeometry>[];
  dragState: EdgeDragState;
  edgeId: string;
  gestureDriver?: AuthoringGraphInteractiveGestureDriver;
  onSessionEnd?: () => void;
  onUpdateBridgeGap?: (bridgeIndex: number, gap: number) => void;
  onUpdateBridgePosition?: (
    bridgeIndex: number,
    segmentIndex: number,
    t: number,
  ) => void;
  onUpdateWaypointHandle?: (
    waypointIndex: number,
    handleKind: "inHandle" | "outHandle",
    independent: boolean,
    position: AuthoringGraphPoint,
  ) => void;
  onUpdateWaypointPosition?: (
    waypointIndex: number,
    position: AuthoringGraphPoint,
  ) => void;
  reactFlow: ReturnType<typeof useReactFlow>;
  session: AuthoringGraphInteractiveGestureSession;
  waypointSegments: ReturnType<typeof buildAuthoringGraphWaypointSegments>;
}

interface AuthoringGraphDirectionMarkerSize {
  height: number;
  width: number;
}

interface AuthoringGraphDirectionMarkerEndpoints {
  source: AuthoringGraphPoint;
  target: AuthoringGraphPoint;
}

let activeEdgeDragSession: ActiveAuthoringGraphEdgeDragSession | null = null;
let removeActiveEdgeDragListeners: (() => void) | null = null;

function endActiveEdgeDragSession(mode: "cancel" | "commit"): void {
  const session = activeEdgeDragSession;
  if (!session) {
    return;
  }

  activeEdgeDragSession = null;

  if (removeActiveEdgeDragListeners) {
    removeActiveEdgeDragListeners();
    removeActiveEdgeDragListeners = null;
  }

  if (mode === "commit") {
    session.session.commitGesture(session.gestureDriver);
  } else {
    session.session.cancelGesture(session.gestureDriver);
  }

  session.onSessionEnd?.();
}

function ensureActiveEdgeDragListeners(): void {
  if (typeof window === "undefined" || removeActiveEdgeDragListeners) {
    return;
  }

  const handlePointerMove = (event: PointerEvent) => {
    const session = activeEdgeDragSession;
    if (!session) {
      return;
    }

    const nextPosition = session.reactFlow.screenToFlowPosition({
      x: event.clientX,
      y: event.clientY,
    });

    if (
      session.dragState.handleKind === "anchor" &&
      typeof session.dragState.waypointIndex === "number"
    ) {
      session.onUpdateWaypointPosition?.(
        session.dragState.waypointIndex,
        nextPosition,
      );
      return;
    }

    if (
      (session.dragState.handleKind === "inHandle" ||
        session.dragState.handleKind === "outHandle") &&
      typeof session.dragState.waypointIndex === "number"
    ) {
      session.onUpdateWaypointHandle?.(
        session.dragState.waypointIndex,
        session.dragState.handleKind,
        event.shiftKey,
        nextPosition,
      );
      return;
    }

    if (
      session.dragState.handleKind === "bridge" &&
      typeof session.dragState.bridgeIndex === "number"
    ) {
      const location = resolveNearestBridgeLocation(
        session.waypointSegments,
        nextPosition,
      );
      if (location) {
        session.onUpdateBridgePosition?.(
          session.dragState.bridgeIndex,
          location.segmentIndex,
          location.t,
        );
      }
      return;
    }

    if (
      session.dragState.handleKind === "bridgeGap" &&
      typeof session.dragState.bridgeIndex === "number"
    ) {
      const geometry = session.bridgeGeometries[session.dragState.bridgeIndex];
      if (geometry) {
        session.onUpdateBridgeGap?.(
          session.dragState.bridgeIndex,
          resolveBridgeGapFromHandle(geometry, nextPosition),
        );
      }
    }
  };

  const handlePointerUp = () => {
    endActiveEdgeDragSession("commit");
  };

  const handlePointerCancel = () => {
    endActiveEdgeDragSession("cancel");
  };

  const handleKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "Escape") {
      return;
    }

    endActiveEdgeDragSession("cancel");
  };

  window.addEventListener("pointermove", handlePointerMove);
  window.addEventListener("pointerup", handlePointerUp);
  window.addEventListener("pointercancel", handlePointerCancel);
  window.addEventListener("keydown", handleKeyDown);
  removeActiveEdgeDragListeners = () => {
    window.removeEventListener("pointermove", handlePointerMove);
    window.removeEventListener("pointerup", handlePointerUp);
    window.removeEventListener("pointercancel", handlePointerCancel);
    window.removeEventListener("keydown", handleKeyDown);
  };
}

function beginActiveEdgeDragSession(
  session: ActiveAuthoringGraphEdgeDragSession,
): void {
  endActiveEdgeDragSession("cancel");
  activeEdgeDragSession = session;
  ensureActiveEdgeDragListeners();
}

function isActiveEdgeDragSession(edgeId: string): boolean {
  return activeEdgeDragSession?.edgeId === edgeId;
}

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
  onInsertWaypoint?: (
    segmentIndex: number,
    segmentT: number,
    waypoint: AuthoringGraphWaypoint,
  ) => void;
  onSelectBridge?: (bridgeIndex: number) => void;
  onSelectWaypoint?: (waypointIndex: number) => void;
  onUpdateBridgeGap?: (bridgeIndex: number, gap: number) => void;
  onUpdateBridgePosition?: (
    bridgeIndex: number,
    segmentIndex: number,
    t: number,
  ) => void;
  onUpdateWaypointHandle?: (
    waypointIndex: number,
    handleKind: "inHandle" | "outHandle",
    independent: boolean,
    position: AuthoringGraphPoint,
  ) => void;
  onUpdateWaypointPosition?: (
    waypointIndex: number,
    position: AuthoringGraphPoint,
  ) => void;
  selection?: AuthoringGraphEdgeSelection;
  showSecondaryPath?: boolean;
  theme?: AuthoringGraphEdgeTheme;
}

const DEFAULT_THEME: Required<AuthoringGraphEdgeTheme> = {
  addButtonClassName: "",
  addButtonShellClassName: "authoring-graph-edge__add-button-shell",
  bridgeButtonClassName: "authoring-graph-edge__bridge-button",
  bridgeButtonGlyphClassName: "authoring-graph-edge__bridge-button-glyph",
  bridgeCapClassName: "authoring-graph-edge__bridge-cap",
  bridgeDragHandleClassName: "authoring-graph-edge-bridge__drag-handle",
  bridgeGapHandleClassName: "authoring-graph-edge-bridge__gap-handle",
  bridgeGapLineClassName: "authoring-graph-edge-bridge__gap-line",
  bridgeGroupClassName: "authoring-graph-edge-bridge",
  bridgeHitTargetClassName: "authoring-graph-edge__bridge-hit-target",
  bridgeSelectedClassName: "authoring-graph-edge-bridge--selected",
  directionMarkerClassName: "",
  directionMarkerGlyphClassName: "",
  interactionClassName: "authoring-graph-edge__interaction",
  pathClassName: "",
  secondaryPathClassName: "",
  selectionOutlineClassName: "authoring-graph-edge__selection-outline",
  waypointAnchorClassName: "authoring-graph-edge-waypoint__anchor",
  waypointGroupClassName: "authoring-graph-edge-waypoint",
  waypointHandleClassName: "authoring-graph-edge-waypoint__handle",
  waypointHandleInClassName: "",
  waypointHandleLineClassName: "authoring-graph-edge-waypoint__handle-line",
  waypointHandleOutClassName: "",
  waypointSelectedClassName: "authoring-graph-edge-waypoint--selected",
};

export function eventTargetMatchesWaypointHoverSurface(
  target: EventTarget | null,
  selector: string,
): boolean {
  if (
    !target ||
    typeof target !== "object" ||
    !("closest" in target) ||
    typeof target.closest !== "function"
  ) {
    return false;
  }

  return Boolean(target.closest(selector));
}

export function stopWaypointSurfacePropagation(
  event: Pick<
    React.SyntheticEvent,
    "preventDefault" | "stopPropagation" | "nativeEvent"
  >,
): void {
  event.preventDefault();
  event.stopPropagation();
  if (
    event.nativeEvent &&
    typeof event.nativeEvent === "object" &&
    "stopImmediatePropagation" in event.nativeEvent &&
    typeof event.nativeEvent.stopImmediatePropagation === "function"
  ) {
    // React propagation is not sufficient here because XYFlow also listens at the native layer.
    event.nativeEvent.stopImmediatePropagation();
  }
}

export function AuthoringGraphEdge({
  addBridgeLabel = "Add bridge",
  addWaypointLabel = "Add waypoint",
  curveOffsetPx,
  directionMarker = false,
  directionMarkerPlacement = "visible-interval",
  directionMarkerTargetBorderRadiusPx = 0,
  gestureDriver,
  layout,
  onInsertBridge,
  onInsertWaypoint,
  onSelectBridge,
  onSelectWaypoint,
  onUpdateBridgeGap,
  onUpdateBridgePosition,
  onUpdateWaypointHandle,
  onUpdateWaypointPosition,
  selection,
  showSecondaryPath = false,
  theme,
  ...props
}: AuthoringGraphEdgeProps): React.JSX.Element {
  const reactFlow = useReactFlow();
  const canvas = useStore((state) => state.domNode);
  const gestureSessionRef = React.useRef(
    createAuthoringGraphInteractiveGestureSession(),
  );
  const isMountedRef = React.useRef(true);
  const resolvedTheme = {
    ...DEFAULT_THEME,
    ...theme,
  };
  const interactionSelector = classNameSelector(
    resolvedTheme.interactionClassName,
  );
  const addButtonSelector = classNameSelector(
    resolvedTheme.addButtonShellClassName,
  );
  const [isBridgeAffordance, setIsBridgeAffordance] = React.useState(false);
  const [hoveredInterval, setHoveredInterval] = React.useState<ReturnType<
    typeof resolveNearestInsertionInterval
  > | null>(null);
  const [dragState, setDragState] = React.useState<EdgeDragState | null>(null);
  const bridges = layout?.bridges ?? [];
  const waypoints = layout?.waypoints ?? [];
  const automaticCurveSegments = React.useMemo(
    () =>
      typeof curveOffsetPx === "number" && waypoints.length === 0
        ? [
            buildAutomaticCurveSegment({
              offsetPx: curveOffsetPx,
              source: { x: props.sourceX, y: props.sourceY },
              target: { x: props.targetX, y: props.targetY },
            }),
          ]
        : null,
    [
      curveOffsetPx,
      props.sourceX,
      props.sourceY,
      props.targetX,
      props.targetY,
      waypoints.length,
    ],
  );
  const waypointSegments = React.useMemo(
    () =>
      buildAuthoringGraphWaypointSegments({
        source: { x: props.sourceX, y: props.sourceY },
        sourcePosition: props.sourcePosition,
        target: { x: props.targetX, y: props.targetY },
        targetPosition: props.targetPosition,
        waypoints,
      }),
    [
      props.sourcePosition,
      props.sourceX,
      props.sourceY,
      props.targetPosition,
      props.targetX,
      props.targetY,
      waypoints,
    ],
  );
  const edgeSegments = automaticCurveSegments ?? waypointSegments;
  const insertionIntervals = React.useMemo(
    () => buildEdgeInsertionIntervals(edgeSegments, bridges),
    [bridges, edgeSegments],
  );
  const waypointPath = React.useMemo(
    () =>
      buildAuthoringGraphWaypointPath({
        source: { x: props.sourceX, y: props.sourceY },
        sourcePosition: props.sourcePosition,
        target: { x: props.targetX, y: props.targetY },
        targetPosition: props.targetPosition,
        waypoints,
      }),
    [
      props.sourcePosition,
      props.sourceX,
      props.sourceY,
      props.targetPosition,
      props.targetX,
      props.targetY,
      waypoints,
    ],
  );
  const [defaultBezierPath] = getBezierPath({
    sourceX: props.sourceX,
    sourceY: props.sourceY,
    sourcePosition: props.sourcePosition,
    targetX: props.targetX,
    targetY: props.targetY,
    targetPosition: props.targetPosition,
  });
  const automaticCurvePath = automaticCurveSegments
    ? buildPathFromSegments(automaticCurveSegments)
    : null;
  const basePath =
    automaticCurvePath ??
    (waypoints.length > 0 ? waypointPath : defaultBezierPath);
  const visiblePath = React.useMemo(
    () =>
      bridges.length > 0
        ? buildVisibleEdgePathWithBridges(edgeSegments, bridges)
        : basePath,
    [basePath, bridges, edgeSegments],
  );
  const selectedBridgeIndex =
    typeof selection?.bridgeIndex === "number" ? selection.bridgeIndex : null;
  const selectedWaypointIndex =
    typeof selection?.waypointIndex === "number"
      ? selection.waypointIndex
      : null;
  const addButtonPosition = hoveredInterval
    ? resolveInsertionIntervalMidpoint(edgeSegments, hoveredInterval)
    : null;
  const bridgeGeometries = React.useMemo(
    () => bridges.map((bridge) => resolveBridgeGeometry(edgeSegments, bridge)),
    [bridges, edgeSegments],
  );
  const [liveDirectionMarkerTick, setLiveDirectionMarkerTick] =
    React.useState(0);
  const liveDirectionMarkerEndpoints = React.useMemo(
    () =>
      directionMarkerPlacement === "target-boundary"
        ? resolveDirectionMarkerDomEndpoints({
            canvas,
            reactFlow,
            sourceNodeId: props.source,
            targetNodeId: props.target,
          })
        : null,
    [
      directionMarkerPlacement,
      liveDirectionMarkerTick,
      canvas,
      props.source,
      props.target,
      reactFlow,
    ],
  );
  const directionMarkerSegments = React.useMemo(() => {
    if (!liveDirectionMarkerEndpoints) {
      return edgeSegments;
    }

    if (typeof curveOffsetPx === "number" && waypoints.length === 0) {
      return [
        buildAutomaticCurveSegment({
          offsetPx: curveOffsetPx,
          source: liveDirectionMarkerEndpoints.source,
          target: liveDirectionMarkerEndpoints.target,
        }),
      ];
    }

    return buildAuthoringGraphWaypointSegments({
      source: liveDirectionMarkerEndpoints.source,
      sourcePosition: props.sourcePosition,
      target: liveDirectionMarkerEndpoints.target,
      targetPosition: props.targetPosition,
      waypoints,
    });
  }, [
    curveOffsetPx,
    edgeSegments,
    liveDirectionMarkerEndpoints,
    props.sourcePosition,
    props.targetPosition,
    waypoints,
  ]);
  const directionMarkerTargetCenter = liveDirectionMarkerEndpoints?.target ?? {
    x: props.targetX,
    y: props.targetY,
  };
  const targetBoundaryFromNode = resolveDirectionMarkerTargetBoundary({
    borderRadiusPx: directionMarkerTargetBorderRadiusPx,
    center: directionMarkerTargetCenter,
    getTargetNode: () => reactFlow.getNode(props.target),
  });
  const targetSizeFromDom = targetBoundaryFromNode
    ? null
    : resolveDirectionMarkerDomTargetSize({
        canvas,
        reactFlow,
        targetNodeId: props.target,
      });
  const targetBoundary =
    targetBoundaryFromNode ??
    (targetSizeFromDom
      ? {
          borderRadiusPx: directionMarkerTargetBorderRadiusPx,
          center: directionMarkerTargetCenter,
          height: targetSizeFromDom.height,
          width: targetSizeFromDom.width,
        }
      : null);
  const directionMarkerGeometry = React.useMemo(() => {
    if (!directionMarker) {
      return null;
    }

    if (directionMarkerPlacement === "target-boundary") {
      return targetBoundary
        ? resolveTargetBoundaryDirectionMarkerGeometry(
            directionMarkerSegments,
            targetBoundary,
          )
        : null;
    }

    return resolveDirectionMarkerGeometry(edgeSegments, bridges);
  }, [
    bridges,
    directionMarker,
    directionMarkerPlacement,
    directionMarkerSegments,
    edgeSegments,
    targetBoundary,
  ]);

  React.useEffect(() => {
    if (
      !directionMarker ||
      directionMarkerPlacement !== "target-boundary" ||
      typeof window === "undefined"
    ) {
      return undefined;
    }

    let frameId: number | null = null;
    const requestMarkerFrame = () => {
      if (frameId !== null) {
        return;
      }

      frameId = window.requestAnimationFrame(() => {
        frameId = null;
        setLiveDirectionMarkerTick((current) => current + 1);
      });
    };
    const handlePointerMove = (event: PointerEvent) => {
      if (event.buttons !== 0) {
        requestMarkerFrame();
      }
    };
    const handlePointerUp = () => {
      requestMarkerFrame();
    };

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
    window.addEventListener("pointercancel", handlePointerUp);
    return () => {
      if (frameId !== null) {
        window.cancelAnimationFrame(frameId);
      }

      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
      window.removeEventListener("pointercancel", handlePointerUp);
    };
  }, [directionMarker, directionMarkerPlacement]);

  React.useEffect(() => {
    isMountedRef.current = true;

    return () => {
      isMountedRef.current = false;
    };
  }, []);

  React.useEffect(
    () => () => {
      if (!isActiveEdgeDragSession(props.id)) {
        gestureSessionRef.current.cancelGesture(gestureDriver);
      }
    },
    [gestureDriver, props.id],
  );

  const isDragging = dragState !== null || isActiveEdgeDragSession(props.id);

  React.useEffect(() => {
    if (!hoveredInterval || isDragging) {
      return;
    }

    const handleKeyChange = (event: KeyboardEvent) => {
      if (event.key === "Shift") {
        setIsBridgeAffordance(event.type === "keydown");
      }
    };

    window.addEventListener("keydown", handleKeyChange);
    window.addEventListener("keyup", handleKeyChange);
    return () => {
      window.removeEventListener("keydown", handleKeyChange);
      window.removeEventListener("keyup", handleKeyChange);
    };
  }, [hoveredInterval, isDragging]);

  const handleEdgeMouseMove = React.useCallback(
    (event: React.MouseEvent<SVGPathElement>) => {
      if (isDragging) {
        return;
      }

      setIsBridgeAffordance(event.shiftKey);
      if (insertionIntervals.length <= 1) {
        setHoveredInterval(insertionIntervals[0] ?? null);
        return;
      }

      const pointer = reactFlow.screenToFlowPosition({
        x: event.clientX,
        y: event.clientY,
      });
      setHoveredInterval(
        resolveNearestInsertionInterval(edgeSegments, bridges, pointer),
      );
    },
    [bridges, edgeSegments, insertionIntervals, isDragging, reactFlow],
  );

  const handleEdgeMouseLeave = React.useCallback(
    (event: React.MouseEvent<SVGPathElement>) => {
      if (
        eventTargetMatchesWaypointHoverSurface(
          event.relatedTarget,
          addButtonSelector,
        )
      ) {
        return;
      }

      if (!isDragging) {
        setIsBridgeAffordance(false);
        setHoveredInterval(null);
      }
    },
    [addButtonSelector, isDragging],
  );

  const handleAddButtonMouseLeave = React.useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      if (
        eventTargetMatchesWaypointHoverSurface(
          event.relatedTarget,
          interactionSelector,
        )
      ) {
        return;
      }

      if (!isDragging) {
        setIsBridgeAffordance(false);
        setHoveredInterval(null);
      }
    },
    [interactionSelector, isDragging],
  );

  const handleWaypointPointerDown = React.useCallback(
    (
      event: React.PointerEvent<SVGCircleElement>,
      waypointIndex: number,
      handleKind: "anchor" | "inHandle" | "outHandle",
    ) => {
      stopWaypointSurfacePropagation(event);
      gestureSessionRef.current.beginGesture(
        gestureDriver,
        resolveAuthoringGraphEdgeGestureLabel(handleKind),
      );
      onSelectWaypoint?.(waypointIndex);
      const nextDragState = {
        handleKind,
        waypointIndex,
      } satisfies EdgeDragState;
      setDragState(nextDragState);
      beginActiveEdgeDragSession({
        bridgeGeometries,
        dragState: nextDragState,
        edgeId: props.id,
        gestureDriver,
        onSessionEnd: () => {
          if (isMountedRef.current) {
            setDragState(null);
          }
        },
        onUpdateBridgeGap,
        onUpdateBridgePosition,
        onUpdateWaypointHandle,
        onUpdateWaypointPosition,
        reactFlow,
        session: gestureSessionRef.current,
        waypointSegments: edgeSegments,
      });
    },
    [
      bridgeGeometries,
      gestureDriver,
      onSelectWaypoint,
      onUpdateBridgeGap,
      onUpdateBridgePosition,
      onUpdateWaypointHandle,
      onUpdateWaypointPosition,
      props.id,
      reactFlow,
      edgeSegments,
    ],
  );

  const handleBridgePointerDown = React.useCallback(
    (
      event: React.PointerEvent<SVGPathElement | SVGCircleElement>,
      bridgeIndex: number,
      handleKind: "bridge" | "bridgeGap",
    ) => {
      stopWaypointSurfacePropagation(event);
      gestureSessionRef.current.beginGesture(
        gestureDriver,
        resolveAuthoringGraphEdgeGestureLabel(handleKind),
      );
      onSelectBridge?.(bridgeIndex);
      const nextDragState = { bridgeIndex, handleKind } satisfies EdgeDragState;
      setDragState(nextDragState);
      beginActiveEdgeDragSession({
        bridgeGeometries,
        dragState: nextDragState,
        edgeId: props.id,
        gestureDriver,
        onSessionEnd: () => {
          if (isMountedRef.current) {
            setDragState(null);
          }
        },
        onUpdateBridgeGap,
        onUpdateBridgePosition,
        onUpdateWaypointHandle,
        onUpdateWaypointPosition,
        reactFlow,
        session: gestureSessionRef.current,
        waypointSegments: edgeSegments,
      });
    },
    [
      bridgeGeometries,
      gestureDriver,
      onSelectBridge,
      onUpdateBridgeGap,
      onUpdateBridgePosition,
      onUpdateWaypointHandle,
      onUpdateWaypointPosition,
      props.id,
      reactFlow,
      edgeSegments,
    ],
  );

  const handleClick = React.useCallback(
    (event: React.MouseEvent<SVGCircleElement | SVGPathElement>) => {
      stopWaypointSurfacePropagation(event);
    },
    [],
  );

  const handleAddWaypoint = React.useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.preventDefault();
      event.stopPropagation();

      if (!hoveredInterval) {
        return;
      }

      setHoveredInterval(null);
      setIsBridgeAffordance(false);

      if (isBridgeAffordance) {
        onInsertBridge?.(
          hoveredInterval.segmentIndex,
          createBridgeForInterval(hoveredInterval),
        );
        return;
      }

      const segment = edgeSegments[hoveredInterval.segmentIndex];
      if (!segment) {
        return;
      }

      onInsertWaypoint?.(
        hoveredInterval.segmentIndex,
        (hoveredInterval.startT + hoveredInterval.endT) * 0.5,
        createAuthoringGraphWaypointForSegmentAt(
          segment,
          (hoveredInterval.startT + hoveredInterval.endT) * 0.5,
        ),
      );
    },
    [
      edgeSegments,
      hoveredInterval,
      isBridgeAffordance,
      onInsertBridge,
      onInsertWaypoint,
    ],
  );

  return (
    <>
      <BaseEdge
        className={resolvedTheme.pathClassName || undefined}
        interactionWidth={0}
        markerEnd={props.markerEnd}
        path={visiblePath}
        pointerEvents="none"
        style={props.style}
      />
      <BaseEdge
        className={joinClassNames(
          "react-flow__edge-interaction",
          resolvedTheme.interactionClassName,
        )}
        interactionWidth={0}
        onMouseLeave={handleEdgeMouseLeave}
        onMouseMove={handleEdgeMouseMove}
        path={basePath}
        style={{
          pointerEvents: "stroke",
          stroke: "currentColor",
          strokeOpacity: 0,
          strokeWidth: props.interactionWidth ?? 20,
        }}
        vectorEffect="non-scaling-stroke"
      />
      {showSecondaryPath && resolvedTheme.secondaryPathClassName ? (
        <path
          className={resolvedTheme.secondaryPathClassName}
          d={visiblePath}
          fill="none"
          pointerEvents="none"
        />
      ) : null}
      {bridges.map((bridge, bridgeIndex) => {
        const geometry = bridgeGeometries[bridgeIndex];
        if (!geometry) {
          return null;
        }

        const isSelected = bridgeIndex === selectedBridgeIndex;
        return (
          <g
            className={joinClassNames(
              resolvedTheme.bridgeGroupClassName,
              isSelected ? resolvedTheme.bridgeSelectedClassName : null,
            )}
            key={`${props.id}-bridge-${bridgeIndex}`}
            pointerEvents="all"
          >
            <path
              className={resolvedTheme.bridgeHitTargetClassName}
              d={`M ${geometry.gapStart.x} ${geometry.gapStart.y} L ${geometry.gapEnd.x} ${geometry.gapEnd.y}`}
              fill="none"
              onClick={handleClick}
              onPointerDown={(event) =>
                handleBridgePointerDown(event, bridgeIndex, "bridge")
              }
              pointerEvents="all"
            />
            <line
              className={joinClassNames(
                "react-flow__edge-path",
                resolvedTheme.bridgeCapClassName,
              )}
              onClick={handleClick}
              onPointerDown={(event) =>
                handleBridgePointerDown(event, bridgeIndex, "bridge")
              }
              pointerEvents="all"
              x1={geometry.startCapStart.x}
              x2={geometry.startCapEnd.x}
              y1={geometry.startCapStart.y}
              y2={geometry.startCapEnd.y}
            />
            <line
              className={joinClassNames(
                "react-flow__edge-path",
                resolvedTheme.bridgeCapClassName,
              )}
              onClick={handleClick}
              onPointerDown={(event) =>
                handleBridgePointerDown(event, bridgeIndex, "bridge")
              }
              pointerEvents="all"
              x1={geometry.endCapStart.x}
              x2={geometry.endCapEnd.x}
              y1={geometry.endCapStart.y}
              y2={geometry.endCapEnd.y}
            />
            {isSelected ? (
              <>
                <line
                  className={resolvedTheme.bridgeGapLineClassName}
                  pointerEvents="none"
                  x1={geometry.center.x}
                  x2={geometry.sizeHandle.x}
                  y1={geometry.center.y}
                  y2={geometry.sizeHandle.y}
                />
                <circle
                  className={resolvedTheme.bridgeGapHandleClassName}
                  cx={geometry.sizeHandle.x}
                  cy={geometry.sizeHandle.y}
                  onClick={handleClick}
                  onPointerDown={(event) =>
                    handleBridgePointerDown(event, bridgeIndex, "bridgeGap")
                  }
                  pointerEvents="all"
                  r="4.5"
                />
              </>
            ) : null}
          </g>
        );
      })}
      {typeof selectedBridgeIndex === "number" &&
      bridgeGeometries[selectedBridgeIndex] ? (
        <circle
          aria-label="Move bridge"
          className={resolvedTheme.bridgeDragHandleClassName}
          cx={bridgeGeometries[selectedBridgeIndex]!.center.x}
          cy={bridgeGeometries[selectedBridgeIndex]!.center.y}
          onClick={handleClick}
          onPointerDown={(event) =>
            handleBridgePointerDown(event, selectedBridgeIndex, "bridge")
          }
          pointerEvents="all"
          r="4.5"
        />
      ) : null}
      {waypoints.map((waypoint, waypointIndex) => {
        const isSelected = waypointIndex === selectedWaypointIndex;
        return (
          <g
            className={joinClassNames(
              resolvedTheme.waypointGroupClassName,
              isSelected ? resolvedTheme.waypointSelectedClassName : null,
            )}
            key={`${props.id}-waypoint-${waypointIndex}`}
            pointerEvents="all"
          >
            {isSelected ? (
              <>
                <line
                  className={resolvedTheme.waypointHandleLineClassName}
                  pointerEvents="none"
                  x1={waypoint.position.x}
                  x2={waypoint.inHandle.x}
                  y1={waypoint.position.y}
                  y2={waypoint.inHandle.y}
                />
                <line
                  className={resolvedTheme.waypointHandleLineClassName}
                  pointerEvents="none"
                  x1={waypoint.position.x}
                  x2={waypoint.outHandle.x}
                  y1={waypoint.position.y}
                  y2={waypoint.outHandle.y}
                />
                <circle
                  className={joinClassNames(
                    resolvedTheme.waypointHandleClassName,
                    resolvedTheme.waypointHandleInClassName,
                  )}
                  cx={waypoint.inHandle.x}
                  cy={waypoint.inHandle.y}
                  onClick={handleClick}
                  onPointerDown={(event) =>
                    handleWaypointPointerDown(event, waypointIndex, "inHandle")
                  }
                  pointerEvents="all"
                  r="4.5"
                />
                <circle
                  className={joinClassNames(
                    resolvedTheme.waypointHandleClassName,
                    resolvedTheme.waypointHandleOutClassName,
                  )}
                  cx={waypoint.outHandle.x}
                  cy={waypoint.outHandle.y}
                  onClick={handleClick}
                  onPointerDown={(event) =>
                    handleWaypointPointerDown(event, waypointIndex, "outHandle")
                  }
                  pointerEvents="all"
                  r="4.5"
                />
              </>
            ) : null}
            <circle
              className={resolvedTheme.waypointAnchorClassName}
              cx={waypoint.position.x}
              cy={waypoint.position.y}
              onClick={handleClick}
              onPointerDown={(event) =>
                handleWaypointPointerDown(event, waypointIndex, "anchor")
              }
              pointerEvents="all"
              r={isSelected ? "6" : "4.5"}
            />
          </g>
        );
      })}
      {addButtonPosition && !isDragging ? (
        <EdgeLabelRenderer>
          <div
            className={joinClassNames(
              resolvedTheme.addButtonShellClassName,
              "nodrag",
              "nopan",
            )}
            onMouseDown={(event) => event.stopPropagation()}
            onMouseLeave={handleAddButtonMouseLeave}
            style={{
              left: `${addButtonPosition.x}px`,
              top: `${addButtonPosition.y}px`,
              transform: "translate(-50%, -50%)",
            }}
          >
            {isBridgeAffordance ? (
              <button
                aria-label={addBridgeLabel}
                className={joinClassNames(
                  "icon-button",
                  "icon-button--ghost",
                  "icon-button--micro",
                  resolvedTheme.bridgeButtonClassName,
                )}
                onClick={handleAddWaypoint}
                title={addBridgeLabel}
                type="button"
              >
                <span
                  aria-hidden="true"
                  className={resolvedTheme.bridgeButtonGlyphClassName}
                >
                  ||
                </span>
              </button>
            ) : (
              <IconButton
                className={resolvedTheme.addButtonClassName || undefined}
                icon="add"
                label={addWaypointLabel}
                mode="micro"
                onClick={handleAddWaypoint}
              />
            )}
          </div>
        </EdgeLabelRenderer>
      ) : null}
      {directionMarkerGeometry && resolvedTheme.directionMarkerClassName ? (
        <EdgeLabelRenderer>
          <div
            aria-hidden="true"
            className={joinClassNames(
              resolvedTheme.directionMarkerClassName,
              "nodrag",
              "nopan",
            )}
            style={{
              left: `${directionMarkerGeometry.position.x}px`,
              top: `${directionMarkerGeometry.position.y}px`,
              transform: `translate(-50%, -50%) rotate(${directionMarkerGeometry.rotationDegrees}deg)`,
            }}
          >
            <span
              aria-hidden="true"
              className={joinClassNames(
                "codicon",
                "codicon-arrow-right",
                resolvedTheme.directionMarkerGlyphClassName,
              )}
            />
          </div>
        </EdgeLabelRenderer>
      ) : null}
      {selectedBridgeIndex !== null || selectedWaypointIndex !== null ? (
        <path
          className={resolvedTheme.selectionOutlineClassName}
          d={visiblePath}
          fill="none"
          pointerEvents="none"
        />
      ) : null}
    </>
  );
}

function joinClassNames(
  ...classNames: Array<string | null | undefined>
): string | undefined {
  const resolved = classNames
    .flatMap((value) => value?.split(/\s+/u) ?? [])
    .filter((value) => value.length > 0)
    .join(" ")
    .trim();

  return resolved.length > 0 ? resolved : undefined;
}

function buildAutomaticCurveSegment({
  offsetPx,
  source,
  target,
}: {
  offsetPx: number;
  source: AuthoringGraphPoint;
  target: AuthoringGraphPoint;
}): ReturnType<typeof buildAuthoringGraphWaypointSegments>[number] {
  const deltaX = target.x - source.x;
  const deltaY = target.y - source.y;
  const length = Math.hypot(deltaX, deltaY) || 1;
  const normal = {
    x: -deltaY / length,
    y: deltaX / length,
  };
  const centre = {
    x: (source.x + target.x) / 2 + normal.x * offsetPx,
    y: (source.y + target.y) / 2 + normal.y * offsetPx,
  };

  return {
    controlA: centre,
    controlB: centre,
    end: target,
    start: source,
  };
}

function buildPathFromSegments(
  segments: readonly ReturnType<
    typeof buildAuthoringGraphWaypointSegments
  >[number][],
): string {
  return segments.reduce((path, segment, index) => {
    const segmentPath = `C ${formatPoint(segment.controlA)}, ${formatPoint(segment.controlB)}, ${formatPoint(segment.end)}`;
    return index === 0
      ? `M ${formatPoint(segment.start)} ${segmentPath}`
      : `${path} ${segmentPath}`;
  }, "");
}

function formatPoint(point: AuthoringGraphPoint): string {
  return `${Number(point.x.toFixed(3))} ${Number(point.y.toFixed(3))}`;
}

function classNameSelector(className: string): string {
  const firstClass = className.split(/\s+/u).find((token) => token.length > 0);
  return firstClass ? `.${firstClass}` : "";
}

function resolveDirectionMarkerTargetBoundary({
  borderRadiusPx,
  center,
  getTargetNode,
}: {
  borderRadiusPx: number;
  center: AuthoringGraphPoint;
  getTargetNode: () => ReturnType<ReturnType<typeof useReactFlow>["getNode"]>;
}): AuthoringGraphDirectionMarkerBoundary | null {
  const targetNode = getTargetNode();
  const width = targetNode?.measured?.width ?? targetNode?.width;
  const height = targetNode?.measured?.height ?? targetNode?.height;

  if (
    typeof width !== "number" ||
    typeof height !== "number" ||
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  ) {
    return null;
  }

  return {
    borderRadiusPx,
    center,
    height,
    width,
  };
}

function resolveDirectionMarkerDomTargetSize({
  canvas,
  reactFlow,
  targetNodeId,
}: {
  canvas: HTMLElement | null;
  reactFlow: ReturnType<typeof useReactFlow>;
  targetNodeId: string;
}): AuthoringGraphDirectionMarkerSize | null {
  if (typeof document === "undefined") {
    return null;
  }

  const targetElement = findReactFlowNodeElement(canvas, targetNodeId);
  if (!targetElement) {
    return null;
  }

  const rect = targetElement.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) {
    return null;
  }

  const left = reactFlow.screenToFlowPosition({ x: rect.left, y: rect.top });
  const right = reactFlow.screenToFlowPosition({ x: rect.right, y: rect.top });
  const top = reactFlow.screenToFlowPosition({ x: rect.left, y: rect.top });
  const bottom = reactFlow.screenToFlowPosition({
    x: rect.left,
    y: rect.bottom,
  });
  const width = Math.abs(right.x - left.x);
  const height = Math.abs(bottom.y - top.y);

  if (
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  ) {
    return null;
  }

  return {
    height,
    width,
  };
}

function resolveDirectionMarkerDomEndpoints({
  canvas,
  reactFlow,
  sourceNodeId,
  targetNodeId,
}: {
  canvas: HTMLElement | null;
  reactFlow: ReturnType<typeof useReactFlow>;
  sourceNodeId: string;
  targetNodeId: string;
}): AuthoringGraphDirectionMarkerEndpoints | null {
  if (typeof document === "undefined") {
    return null;
  }

  const sourceElement = findReactFlowNodeElement(canvas, sourceNodeId);
  const targetElement = findReactFlowNodeElement(canvas, targetNodeId);
  if (!sourceElement || !targetElement) {
    return null;
  }

  const sourceRect = sourceElement.getBoundingClientRect();
  const targetRect = targetElement.getBoundingClientRect();
  if (
    sourceRect.width <= 0 ||
    sourceRect.height <= 0 ||
    targetRect.width <= 0 ||
    targetRect.height <= 0
  ) {
    return null;
  }

  return {
    source: reactFlow.screenToFlowPosition({
      x: sourceRect.left + sourceRect.width * 0.5,
      y: sourceRect.top + sourceRect.height * 0.5,
    }),
    target: reactFlow.screenToFlowPosition({
      x: targetRect.left + targetRect.width * 0.5,
      y: targetRect.top + targetRect.height * 0.5,
    }),
  };
}

export function findReactFlowNodeElement(
  canvas: HTMLElement | null,
  nodeId: string,
): HTMLElement | null {
  return (
    Array.from(
      canvas?.querySelectorAll<HTMLElement>(".react-flow__node") ?? [],
    ).find((element) => element.getAttribute("data-id") === nodeId) ?? null
  );
}
