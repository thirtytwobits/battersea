/**
 * Copyright (c) Scott A Dixon
 *
 * Implements flow canvas node for the editor's dataflow workspace.
 */
import React from "react";
import {
  Position,
  useUpdateNodeInternals,
  type Node,
  type NodeProps,
} from "@xyflow/react";
import { GraphBadge as Badge } from "../graph/presentation.js";
import { GraphIconButton as IconButton } from "../graph/presentation.js";
import { AuthoringGraphHandle } from "../graph.js";
const useUiRuntimeClientLayoutEffect =
  typeof window === "undefined" ? React.useEffect : React.useLayoutEffect;

import {
  normalizeFlowControllerPortPlacement,
  type FlowControllerPortPlacement,
  resolveFlowControllerPortVisualSide,
} from "./flow-controller-port-placement.js";
import type { FlowStudioPortSelectionTarget } from "./dataflow-editor-state.js";
import type { FlowStudioNodeData } from "./flow-drag.js";
import {
  resolveFlowPortMoveControlState,
  type FlowPortMoveDirection,
} from "./flow-port-move-controls.js";
import {
  formatFlowDefinitionTitle,
  getNodeClassLabel,
} from "./flow-node-definitions.js";
import {
  buildFlowPortSlots,
  resolveFlowStudioResolvedPorts,
  type FlowPortSide,
  type FlowPortSideCounts,
  type FlowStudioResolvedPort,
} from "./flow-node-ports.js";

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

export function resolveFlowStudioCanvasNodeViewModel(
  data: Partial<FlowStudioNodeData> | null | undefined,
): FlowStudioCanvasNodeViewModel {
  const nodeClass = data?.nodeClass;
  const actionPorts = Array.isArray(data?.actionPorts)
    ? resolveFlowStudioResolvedPorts(data.actionPorts, "action", nodeClass)
    : [];
  const automationPorts = Array.isArray(data?.automationPorts)
    ? resolveFlowStudioResolvedPorts(
        data.automationPorts,
        "automation",
        nodeClass,
      )
    : [];
  const inputPorts = Array.isArray(data?.inputPorts)
    ? resolveFlowStudioResolvedPorts(data.inputPorts, "input", nodeClass)
    : [];
  const outputPorts = Array.isArray(data?.outputPorts)
    ? resolveFlowStudioResolvedPorts(data.outputPorts, "output", nodeClass)
    : [];
  const signalPorts = Array.isArray(data?.signalPorts)
    ? resolveFlowStudioResolvedPorts(data.signalPorts, "signal", nodeClass)
    : [];
  const sideCounts: FlowPortSideCounts = {
    actionCount: actionPorts.length,
    automationCount: automationPorts.length,
    inputCount: inputPorts.length,
    outputCount: outputPorts.length,
    signalCount: signalPorts.length,
  };
  const actionPortLayout = buildFlowPortSlots("action", sideCounts, nodeClass);
  const automationPortLayout = buildFlowPortSlots(
    "automation",
    sideCounts,
    nodeClass,
  );
  const inputPortLayout = buildFlowPortSlots("input", sideCounts, nodeClass);
  const outputPortLayout = buildFlowPortSlots("output", sideCounts, nodeClass);
  const signalPortLayout = buildFlowPortSlots("signal", sideCounts, nodeClass);
  const controllerPortPlacement = normalizeFlowControllerPortPlacement(
    data?.controllerPortPlacement,
  );

  return {
    actionPorts,
    actionPortSlots: actionPortLayout.slots,
    automationPorts,
    automationPortSlots: automationPortLayout.slots,
    controllerPortPlacement,
    canonicalDefinitionName:
      typeof data?.definitionName === "string" && data.definitionName.trim()
        ? data.definitionName
        : "UnknownType",
    definitionName:
      typeof data?.definitionName === "string" && data.definitionName.trim()
        ? formatFlowDefinitionTitle(data.definitionName)
        : "Unknown Type",
    hasController: data?.hasController === true,
    inputPorts,
    inputPortSlots: inputPortLayout.slots,
    instanceName:
      typeof data?.instanceName === "string" && data.instanceName.trim()
        ? data.instanceName
        : "Untitled node",
    layout: {
      handleGapPx: actionPortLayout.handleGapPx,
      nodeHeightPx: actionPortLayout.nodeHeightPx,
      nodeWidthPx: actionPortLayout.nodeWidthPx,
      pillMaxWidthPx: actionPortLayout.pillMaxWidthPx,
    },
    nodeClass:
      nodeClass === "source" ||
      nodeClass === "control" ||
      nodeClass === "instrument" ||
      nodeClass === "logic" ||
      nodeClass === "hybrid" ||
      nodeClass === "inline" ||
      nodeClass === "sink"
        ? nodeClass
        : "inline",
    outputPorts,
    outputPortSlots: outputPortLayout.slots,
    reversed: nodeClass === "logic" && controllerPortPlacement === "swapped",
    signalPorts,
    signalPortSlots: signalPortLayout.slots,
  };
}

export function FlowStudioCanvasNode({
  data,
  id,
  selected,
}: NodeProps<FlowStudioCanvasNodeType>): React.JSX.Element {
  const updateNodeInternals = useUpdateNodeInternals();
  const viewModel = resolveFlowStudioCanvasNodeViewModel(data);
  const frameStyle = {
    "--flow-studio-node-height": `${viewModel.layout.nodeHeightPx}px`,
    "--flow-studio-node-port-gap": `${viewModel.layout.handleGapPx}px`,
    "--flow-studio-node-port-pill-width": `${viewModel.layout.pillMaxWidthPx}px`,
    "--flow-studio-node-width": `${viewModel.layout.nodeWidthPx}px`,
  } as React.CSSProperties;

  useUiRuntimeClientLayoutEffect(() => {
    updateNodeInternals(id);
  }, [
    id,
    updateNodeInternals,
    viewModel.actionPorts.map((port) => port.id).join("|"),
    viewModel.automationPorts.map((port) => port.id).join("|"),
    viewModel.controllerPortPlacement,
    viewModel.inputPorts.map((port) => port.id).join("|"),
    viewModel.outputPorts.map((port) => port.id).join("|"),
    viewModel.signalPorts.map((port) => port.id).join("|"),
  ]);

  return (
    <div
      className="flow-studio-node-frame"
      data-node-class={viewModel.nodeClass}
      data-selected={selected ? "true" : "false"}
      style={frameStyle}
    >
      {(() => {
        const actionVisualSide = resolveFlowControllerPortVisualSide({
          nodeClass: viewModel.nodeClass,
          placement: viewModel.controllerPortPlacement,
          side: "action",
        });
        return renderFlowStudioCanvasNodePortControls({
          labelClassName: `flow-studio-node-port-label flow-studio-node-port-label--${actionVisualSide === "left" ? "input" : actionVisualSide === "right" ? "output" : actionVisualSide}`,
          controllerPortPlacement: viewModel.controllerPortPlacement,
          movedPortPulse: data.canvasMovedPortPulse ?? null,
          nodeClass: viewModel.nodeClass,
          nodeId: id,
          onMovePort: data.canvasOnMovePort,
          onPortSelect: data.canvasOnPortSelect,
          ports: viewModel.actionPorts,
          position:
            actionVisualSide === "left"
              ? Position.Left
              : actionVisualSide === "right"
                ? Position.Right
                : actionVisualSide === "bottom"
                  ? Position.Bottom
                  : Position.Top,
          selectedPort: data.canvasSelectedPort ?? null,
          side: "action",
          slots: viewModel.actionPortSlots,
          type: "target",
          visualSide:
            actionVisualSide === "left"
              ? "input"
              : actionVisualSide === "right"
                ? "output"
                : actionVisualSide,
        });
      })()}
      {renderFlowStudioCanvasNodePortControls({
        labelClassName:
          "flow-studio-node-port-label flow-studio-node-port-label--input",
        controllerPortPlacement: viewModel.controllerPortPlacement,
        visualSide: "input",
        movedPortPulse: data.canvasMovedPortPulse ?? null,
        nodeClass: viewModel.nodeClass,
        nodeId: id,
        onMovePort: data.canvasOnMovePort,
        onPortSelect: data.canvasOnPortSelect,
        ports: viewModel.inputPorts,
        position: Position.Left,
        selectedPort: data.canvasSelectedPort ?? null,
        side: "input",
        slots: viewModel.inputPortSlots,
        type: "target",
      })}
      {renderFlowStudioCanvasNodePortControls({
        labelClassName:
          "flow-studio-node-port-label flow-studio-node-port-label--output",
        controllerPortPlacement: viewModel.controllerPortPlacement,
        visualSide: "output",
        movedPortPulse: data.canvasMovedPortPulse ?? null,
        nodeClass: viewModel.nodeClass,
        nodeId: id,
        onMovePort: data.canvasOnMovePort,
        onPortSelect: data.canvasOnPortSelect,
        ports: viewModel.outputPorts,
        position: Position.Right,
        selectedPort: data.canvasSelectedPort ?? null,
        side: "output",
        slots: viewModel.outputPortSlots,
        type: "source",
      })}
      {(() => {
        const signalVisualSide = resolveFlowControllerPortVisualSide({
          nodeClass: viewModel.nodeClass,
          placement: viewModel.controllerPortPlacement,
          side: "signal",
        });
        return renderFlowStudioCanvasNodePortControls({
          labelClassName: `flow-studio-node-port-label flow-studio-node-port-label--${signalVisualSide === "left" ? "input" : signalVisualSide === "right" ? "output" : signalVisualSide}`,
          controllerPortPlacement: viewModel.controllerPortPlacement,
          movedPortPulse: data.canvasMovedPortPulse ?? null,
          nodeClass: viewModel.nodeClass,
          nodeId: id,
          onMovePort: data.canvasOnMovePort,
          onPortSelect: data.canvasOnPortSelect,
          ports: viewModel.signalPorts,
          position:
            signalVisualSide === "left"
              ? Position.Left
              : signalVisualSide === "right"
                ? Position.Right
                : signalVisualSide === "bottom"
                  ? Position.Bottom
                  : Position.Top,
          selectedPort: data.canvasSelectedPort ?? null,
          side: "signal",
          slots: viewModel.signalPortSlots,
          type: "source",
          visualSide:
            signalVisualSide === "left"
              ? "input"
              : signalVisualSide === "right"
                ? "output"
                : signalVisualSide,
        });
      })()}
      {(() => {
        const automationVisualSide = resolveFlowControllerPortVisualSide({
          nodeClass: viewModel.nodeClass,
          placement: viewModel.controllerPortPlacement,
          side: "automation",
        });
        return renderFlowStudioCanvasNodePortControls({
          labelClassName: `flow-studio-node-port-label flow-studio-node-port-label--${automationVisualSide === "left" ? "input" : automationVisualSide === "right" ? "output" : automationVisualSide}`,
          controllerPortPlacement: viewModel.controllerPortPlacement,
          movedPortPulse: data.canvasMovedPortPulse ?? null,
          nodeClass: viewModel.nodeClass,
          nodeId: id,
          onMovePort: data.canvasOnMovePort,
          onPortSelect: data.canvasOnPortSelect,
          ports: viewModel.automationPorts,
          position:
            automationVisualSide === "left"
              ? Position.Left
              : automationVisualSide === "right"
                ? Position.Right
                : automationVisualSide === "bottom"
                  ? Position.Bottom
                  : Position.Top,
          selectedPort: data.canvasSelectedPort ?? null,
          side: "automation",
          slots: viewModel.automationPortSlots,
          type: "target",
          visualSide:
            automationVisualSide === "left"
              ? "input"
              : automationVisualSide === "right"
                ? "output"
                : automationVisualSide,
        });
      })()}
      {viewModel.nodeClass === "logic" ? (
        <LogicNodeSymbol
          definitionName={viewModel.canonicalDefinitionName}
          displayName={viewModel.definitionName}
          instanceName={viewModel.instanceName}
          layout={viewModel.layout}
          parameterValues={data.parameterValues ?? {}}
          placement={viewModel.controllerPortPlacement}
        />
      ) : (
        <div className="flow-studio-node-shell">
          <div className="flow-studio-node-shell__header">
            <div className="flow-studio-node-shell__eyebrow">
              {getNodeClassLabel(viewModel.nodeClass)}
            </div>
            {viewModel.hasController ? (
              <Badge
                classifier="controller"
                className="flow-studio-node-shell__badge"
                helpText="Controller"
                icon="window"
                label="Controller"
                shape="icon"
              />
            ) : null}
          </div>
          <div className="flow-studio-node-shell__title">
            {viewModel.instanceName}
          </div>
          <div className="flow-studio-node-shell__description">
            {viewModel.definitionName}
          </div>
        </div>
      )}
    </div>
  );
}

function LogicNodeSymbol(options: {
  definitionName: string;
  displayName: string;
  instanceName: string;
  layout: FlowStudioCanvasNodeViewModel["layout"];
  parameterValues: Record<string, unknown>;
  placement: FlowControllerPortPlacement;
}): React.JSX.Element {
  const descriptor = resolveLogicGateDescriptor(
    options.definitionName,
    options.parameterValues,
  );
  const labelId = `flow-logic-node-title-${options.definitionName}-${descriptor.label}`;
  const glowFilterId = `${labelId}-glow`;
  const description = `${options.instanceName}, ${options.displayName}`;
  const viewBoxWidth = options.layout.nodeWidthPx;
  const viewBoxHeight = options.layout.nodeHeightPx;
  const reversed = options.placement === "swapped";
  const shapeTransform = reversed
    ? `translate(${viewBoxWidth} 0) scale(-1 1)`
    : undefined;

  return (
    <div
      aria-label={description}
      className="flow-studio-logic-node"
      data-exclusive={descriptor.exclusive ? "true" : "false"}
      data-gate-kind={descriptor.kind}
      data-inverted={descriptor.inverted ? "true" : "false"}
      data-logic-label={descriptor.label}
      data-visual-reversed={reversed ? "true" : "false"}
      role="img"
      title={description}
    >
      <svg
        aria-labelledby={labelId}
        className="flow-studio-logic-node__svg"
        focusable="false"
        viewBox={`0 0 ${viewBoxWidth} ${viewBoxHeight}`}
      >
        <title id={labelId}>{description}</title>
        <defs>
          <filter
            id={glowFilterId}
            x="-45%"
            y="-45%"
            width="190%"
            height="190%"
            colorInterpolationFilters="sRGB"
          >
            <feGaussianBlur in="SourceGraphic" result="blur" stdDeviation="7" />
            <feFlood
              floodColor="currentColor"
              floodOpacity="0.82"
              result="glow-colour"
            />
            <feComposite
              in="glow-colour"
              in2="blur"
              operator="in"
              result="glow"
            />
            <feMerge>
              <feMergeNode in="glow" />
            </feMerge>
          </filter>
        </defs>
        <g
          className="flow-studio-logic-node__glow"
          filter={`url(#${glowFilterId})`}
          transform={shapeTransform}
        >
          <LogicNodeSymbolShape
            descriptor={descriptor}
            height={viewBoxHeight}
            mode="glow"
            width={viewBoxWidth}
          />
        </g>
        <LogicNodeSymbolShape
          descriptor={descriptor}
          height={viewBoxHeight}
          mode="shape"
          transform={shapeTransform}
          width={viewBoxWidth}
        />
        <text
          className="flow-studio-logic-node__label"
          dominantBaseline="middle"
          textAnchor="middle"
          x={viewBoxWidth / 2}
          y={viewBoxHeight / 2}
        >
          {descriptor.label}
        </text>
      </svg>
    </div>
  );
}

function LogicNodeSymbolShape(options: {
  descriptor: LogicGateDescriptor;
  height: number;
  mode: "glow" | "shape";
  transform?: string;
  width: number;
}): React.JSX.Element {
  const { descriptor, height, mode, transform, width } = options;
  const shapeClassName =
    mode === "glow"
      ? "flow-studio-logic-node__glow-shape"
      : "flow-studio-logic-node__shape";
  const bubbleClassName =
    mode === "glow"
      ? "flow-studio-logic-node__glow-bubble"
      : "flow-studio-logic-node__bubble";
  const exclusiveClassName =
    mode === "glow"
      ? "flow-studio-logic-node__glow-shape flow-studio-logic-node__glow-shape--exclusive"
      : "flow-studio-logic-node__shape flow-studio-logic-node__shape--exclusive";
  const centreY = height / 2;
  if (descriptor.kind === "and") {
    const top = Math.max(24, centreY - 34);
    const bottom = Math.min(height - 24, centreY + 34);
    const left = 0;
    const right = descriptor.inverted ? width - 30 : width;
    const curveStart = Math.max(left + 58, right - 72);
    return (
      <>
        <path
          className={shapeClassName}
          d={`M${left} ${top} H${curveStart} C${right - 28} ${top} ${right} ${centreY - 19} ${right} ${centreY} C${right} ${centreY + 19} ${right - 28} ${bottom} ${curveStart} ${bottom} H${left} Z`}
          transform={transform}
        />
        {descriptor.inverted ? (
          <circle
            className={bubbleClassName}
            cx={width - 22}
            cy={centreY}
            r="8"
            transform={transform}
          />
        ) : null}
      </>
    );
  }

  if (descriptor.kind === "or") {
    const top = Math.max(24, centreY - 34);
    const bottom = Math.min(height - 24, centreY + 34);
    const left = 0;
    const right = descriptor.inverted ? width - 30 : width;
    return (
      <>
        <path
          className={shapeClassName}
          d={`M${left} ${top} C${left + 35} ${top + 1} ${right - 30} ${centreY - 24} ${right} ${centreY} C${right - 30} ${centreY + 24} ${left + 35} ${bottom - 1} ${left} ${bottom} C${left + 18} ${centreY + 14} ${left + 18} ${centreY - 14} ${left} ${top} Z`}
          transform={transform}
        />
        {descriptor.exclusive ? (
          <path
            className={exclusiveClassName}
            d={`M${left + 10} ${top} C${left + 27} ${centreY - 15} ${left + 27} ${centreY + 15} ${left + 10} ${bottom}`}
            transform={transform}
          />
        ) : null}
        {descriptor.inverted ? (
          <circle
            className={bubbleClassName}
            cx={width - 22}
            cy={centreY}
            r="8"
            transform={transform}
          />
        ) : null}
      </>
    );
  }

  if (descriptor.kind === "invert") {
    const right = width - 30;
    return (
      <>
        <path
          className={shapeClassName}
          d={`M0 18 V${height - 18} L${right} ${centreY} Z`}
          transform={transform}
        />
        <circle
          className={bubbleClassName}
          cx={width - 22}
          cy={centreY}
          r="8"
          transform={transform}
        />
      </>
    );
  }

  if (descriptor.kind === "mux") {
    const path = buildRoutingTrapezoidPath({
      height,
      multipleSide: "right",
      width,
    });

    return <path className={shapeClassName} d={path} transform={transform} />;
  }

  const path = buildRoutingTrapezoidPath({
    height,
    multipleSide: "left",
    width,
  });

  return <path className={shapeClassName} d={path} transform={transform} />;
}

function buildRoutingTrapezoidPath(options: {
  height: number;
  multipleSide: "left" | "right";
  width: number;
}): string {
  const verticalPadding = 12;
  const narrowInset = 10;
  const wideTop = verticalPadding;
  const wideBottom = options.height - verticalPadding;
  const wideHeight = wideBottom - wideTop;
  const narrowHeight = Math.max(56, wideHeight - narrowInset * 2);
  const narrowTop = (options.height - narrowHeight) / 2;
  const narrowBottom = narrowTop + narrowHeight;
  const left = 0;
  const right = options.width;

  if (options.multipleSide === "right") {
    return `M${left} ${narrowTop} L${right} ${wideTop} V${wideBottom} L${left} ${narrowBottom} Z`;
  }

  return `M${left} ${wideTop} L${right} ${narrowTop} V${narrowBottom} L${left} ${wideBottom} Z`;
}

interface LogicGateDescriptor {
  exclusive: boolean;
  inverted: boolean;
  kind: "and" | "demux" | "invert" | "mux" | "or";
  label:
    | "AND"
    | "DEMUX"
    | "MUX"
    | "NAND"
    | "NOR"
    | "NOT"
    | "OR"
    | "XNOR"
    | "XOR";
}

function resolveLogicGateDescriptor(
  definitionName: string,
  parameterValues: Record<string, unknown>,
): LogicGateDescriptor {
  const label = resolveLogicGateLabel(definitionName, parameterValues);
  const kind =
    label === "MUX"
      ? "mux"
      : label === "DEMUX"
        ? "demux"
        : label === "NOT"
          ? "invert"
          : label.includes("AND")
            ? "and"
            : "or";
  const inverted = label === "NAND" || label === "NOR" || label === "XNOR";
  const exclusive = label === "XOR" || label === "XNOR";

  return {
    exclusive,
    inverted,
    kind,
    label,
  };
}

function resolveLogicGateLabel(
  definitionName: string,
  parameterValues: Record<string, unknown>,
): LogicGateDescriptor["label"] {
  const inverted = parameterValues.not === true;
  const exclusive = parameterValues.exclusive === true;
  if (definitionName === "SignalMultiplexer") {
    return "MUX";
  }
  if (definitionName === "SignalDemultiplexer") {
    return "DEMUX";
  }
  if (definitionName === "AndGate") {
    return inverted ? "NAND" : "AND";
  }
  if (definitionName === "OrGate") {
    if (exclusive && inverted) {
      return "XNOR";
    }
    if (exclusive) {
      return "XOR";
    }
    return inverted ? "NOR" : "OR";
  }
  if (definitionName === "InvertGate") {
    return "NOT";
  }
  return "OR";
}

function resolveLogicPortCompactLabel(
  port: FlowStudioResolvedPort,
  index: number,
): string {
  if (port.id === "input") {
    return "IN";
  }
  if (port.id === "enable") {
    return "EN";
  }

  const indexedPort = /(?:input|output)[_-](\d+)$/i.exec(port.id);
  if (indexedPort?.[1]) {
    return indexedPort[1];
  }

  if (port.side === "action") {
    return `I${index}`;
  }
  if (port.side === "signal") {
    return `O${index}`;
  }

  return port.label;
}

function renderFlowStudioCanvasNodePortControls(options: {
  labelClassName: string;
  movedPortPulse: {
    nodeId: string;
    portId: string;
    replay: "a" | "b";
    side: FlowPortSide;
  } | null;
  controllerPortPlacement: FlowStudioNodeData["controllerPortPlacement"];
  nodeClass: FlowStudioNodeData["nodeClass"];
  nodeId: string;
  onMovePort?: (target: {
    direction: FlowPortMoveDirection;
    nodeId: string;
    portId: string;
    side: FlowPortSide;
  }) => void;
  onPortSelect?: (target: {
    nodeId: string;
    portId: string;
    side: FlowStudioPortSelectionTarget["side"];
  }) => void;
  ports: readonly FlowStudioResolvedPort[];
  position: Position;
  selectedPort?: FlowStudioPortSelectionTarget | null;
  side: FlowPortSide;
  slots: ReturnType<typeof buildFlowPortSlots>["slots"];
  type: "source" | "target";
  visualSide: "bottom" | "input" | "output" | "top";
}): React.JSX.Element[] {
  return options.ports.flatMap((port, index) => {
    const slot = options.slots[index];
    if (!port) {
      return [];
    }
    if (!slot) {
      return [];
    }

    const tooltipText = resolveFlowPortTooltipText(
      port,
      `${port.side} ${index}`,
    );
    const selectedState = isSelectedPort(
      options.selectedPort,
      options.nodeId,
      options.side,
      port.id,
    );
    const moveState =
      selectedState === "true"
        ? resolveFlowPortMoveControlState({
            controllerPortPlacement: options.controllerPortPlacement,
            nodeClass: options.nodeClass,
            portId: port.id,
            ports: options.ports,
            side: options.side,
          })
        : null;
    const pulseReplay = resolveMovedPortPulseReplay(
      options.movedPortPulse,
      options.nodeId,
      options.side,
      port.id,
    );
    const labelStyle =
      options.position === Position.Left || options.position === Position.Right
        ? { top: `${slot.offsetPercent}%` }
        : { left: `${slot.offsetPercent}%` };
    const portButtonId = `flow-studio-node-port-select-${options.nodeId}-${options.side}-${port.id}`;
    const handleClassName = `flow-studio-node-port flow-studio-node-port--${options.visualSide}`;
    const labelClassName = `${options.labelClassName} flow-studio-node-port-label--${options.visualSide}`;
    const compactPortLabel =
      options.nodeClass === "logic"
        ? resolveLogicPortCompactLabel(port, index)
        : port.label;
    const descriptor = {
      direction: options.type,
      family:
        options.side === "action" || options.side === "signal"
          ? "signal"
          : "token",
      handleId: slot.handleId,
      label: port.label,
      orderIndex: slot.index,
      side:
        options.position === Position.Top
          ? "top"
          : options.position === Position.Right
            ? "right"
            : options.position === Position.Bottom
              ? "bottom"
              : "left",
    } as const;

    return [
      <React.Fragment key={`${options.side}-${port.id}`}>
        <AuthoringGraphHandle
          className={handleClassName}
          data-port-display-class={port.displayClass ?? "inline"}
          descriptor={descriptor}
          data-selected={selectedState}
          isConnectable
          onClick={(event) =>
            handlePortSelect(
              event,
              options.onPortSelect,
              options.nodeId,
              options.side,
              port,
              portButtonId,
            )
          }
          style={labelStyle}
          title={tooltipText}
        />
        <span className={labelClassName} style={labelStyle}>
          <span
            className="flow-studio-node-port-cluster"
            data-side={options.side}
          >
            {moveState?.kind === "reorder" ? (
              <IconButton
                className="flow-studio-node-port-move"
                disabled={!moveState.canMoveTowardStart}
                icon={moveState.startIcon}
                label={moveState.startLabel}
                mode="micro"
                onClick={(event) =>
                  handlePortMove(
                    event,
                    options.onMovePort,
                    "toward-start",
                    options.nodeId,
                    options.side,
                    port.id,
                  )
                }
                variant="ghost"
              />
            ) : null}
            {moveState?.kind === "swap-side" ? (
              <IconButton
                className="flow-studio-node-port-move"
                icon={moveState.icon}
                label={moveState.label}
                mode="micro"
                onClick={(event) =>
                  handlePortMove(
                    event,
                    options.onMovePort,
                    moveState.direction,
                    options.nodeId,
                    options.side,
                    port.id,
                  )
                }
                variant="ghost"
              />
            ) : null}
            <button
              aria-label={port.label}
              className="flow-studio-node-port-select"
              data-move-pulse={pulseReplay ?? "none"}
              data-port-display-class={port.displayClass ?? "inline"}
              data-selected={selectedState}
              id={portButtonId}
              onClick={(event) =>
                handlePortSelect(
                  event,
                  options.onPortSelect,
                  options.nodeId,
                  options.side,
                  port,
                  portButtonId,
                )
              }
              onKeyDown={(event) => {
                if (
                  event.key !== "ArrowUp" &&
                  event.key !== "ArrowDown" &&
                  event.key !== "ArrowLeft" &&
                  event.key !== "ArrowRight"
                ) {
                  return;
                }

                event.preventDefault();
                event.stopPropagation();
                if (!moveState) {
                  return;
                }
                const towardStart =
                  event.key === "ArrowUp" || event.key === "ArrowLeft";
                const towardEnd =
                  event.key === "ArrowDown" || event.key === "ArrowRight";
                if (
                  moveState.kind === "swap-side" &&
                  ((moveState.direction === "toward-start" && !towardStart) ||
                    (moveState.direction === "toward-end" && !towardEnd))
                ) {
                  return;
                }

                handlePortMove(
                  event,
                  options.onMovePort,
                  towardStart ? "toward-start" : "toward-end",
                  options.nodeId,
                  options.side,
                  port.id,
                );
              }}
              title={tooltipText}
              type="button"
            >
              {options.nodeClass === "logic" ? (
                <>
                  <span className="flow-studio-node-port-select__compact">
                    {compactPortLabel}
                  </span>
                  <span className="flow-studio-node-port-select__full">
                    {port.label}
                  </span>
                </>
              ) : (
                port.label
              )}
            </button>
            {moveState?.kind === "reorder" ? (
              <IconButton
                className="flow-studio-node-port-move"
                disabled={!moveState.canMoveTowardEnd}
                icon={moveState.endIcon}
                label={moveState.endLabel}
                mode="micro"
                onClick={(event) =>
                  handlePortMove(
                    event,
                    options.onMovePort,
                    "toward-end",
                    options.nodeId,
                    options.side,
                    port.id,
                  )
                }
                variant="ghost"
              />
            ) : null}
          </span>
        </span>
      </React.Fragment>,
    ];
  });
}

export function handlePortSelect(
  event: React.MouseEvent | React.PointerEvent,
  onPortSelect:
    | ((target: {
        nodeId: string;
        portId: string;
        side: FlowStudioPortSelectionTarget["side"];
      }) => void)
    | undefined,
  nodeId: string,
  side: FlowStudioPortSelectionTarget["side"],
  port: FlowStudioResolvedPort | undefined,
  focusTargetId?: string,
): void {
  event.stopPropagation();
  if (!onPortSelect || !port) {
    return;
  }

  onPortSelect({
    nodeId,
    portId: port.id,
    side,
  });
  resolveFlowPortSelectionFocusTarget(
    event.currentTarget,
    focusTargetId,
  )?.focus();
}

function handlePortMove(
  event:
    | React.MouseEvent<HTMLButtonElement>
    | React.KeyboardEvent<HTMLButtonElement>,
  onMovePort:
    | ((target: {
        direction: FlowPortMoveDirection;
        nodeId: string;
        portId: string;
        side: FlowPortSide;
      }) => void)
    | undefined,
  direction: FlowPortMoveDirection,
  nodeId: string,
  side: FlowPortSide,
  portId: string,
): void {
  event.preventDefault();
  event.stopPropagation();
  if (!onMovePort) {
    return;
  }

  onMovePort({
    direction,
    nodeId,
    portId,
    side,
  });
}

export function resolveFlowPortTooltipText(
  port: FlowStudioResolvedPort | undefined,
  fallbackLabel: string,
): string {
  const shortDescription = port?.shortDescription?.trim();
  if (shortDescription) {
    return shortDescription;
  }

  const label = port?.label?.trim();
  return label || fallbackLabel;
}

function isSelectedPort(
  selectedPort: FlowStudioPortSelectionTarget | null | undefined,
  nodeId: string,
  side: FlowStudioPortSelectionTarget["side"],
  portId: string | undefined,
): "true" | "false" {
  return selectedPort?.kind === "port" &&
    selectedPort.nodeId === nodeId &&
    selectedPort.side === side &&
    selectedPort.portId === portId
    ? "true"
    : "false";
}

function resolveFlowPortSelectionFocusTarget(
  currentTarget: EventTarget | null,
  focusTargetId?: string,
): { focus: () => void } | null {
  if (focusTargetId && typeof document !== "undefined") {
    const target = document.getElementById(focusTargetId);
    if (isFocusableTarget(target)) {
      return target;
    }
  }

  return isFocusableTarget(currentTarget) ? currentTarget : null;
}

function isFocusableTarget(value: unknown): value is { focus: () => void } {
  return (
    typeof value === "object" &&
    value !== null &&
    "focus" in value &&
    typeof value.focus === "function"
  );
}

function resolveMovedPortPulseReplay(
  movedPortPulse: {
    nodeId: string;
    portId: string;
    replay: "a" | "b";
    side: FlowPortSide;
  } | null,
  nodeId: string,
  side: FlowPortSide,
  portId: string,
): "a" | "b" | null {
  return movedPortPulse &&
    movedPortPulse.nodeId === nodeId &&
    movedPortPulse.side === side &&
    movedPortPulse.portId === portId
    ? movedPortPulse.replay
    : null;
}
