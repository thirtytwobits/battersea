/**
 * Copyright (c) Scott A Dixon
 *
 * Persists and restores flow persistence for the editor's dataflow workspace.
 */
import type { Node } from "@xyflow/react";
import { createFlowExecutionPolicy, FLOW_DOCUMENT_VERSION, requireSupportedFlowVersion } from "@battersea/flow";
import type {
  FlowDocument as WireFlowDocument,
  FlowEdge as WireFlowEdge,
  FlowNode as WireFlowNode,
  FlowNodeDefinition as WireFlowNodeDefinition,
} from "@battersea/flow";
import type {
  FlowStudioEdgeBridge,
  FlowStudioEdge,
  FlowStudioEdgeWaypoint,
  FlowStudioSelectionTarget,
} from "./dataflow-editor-state.js";
import type { FlowStudioNodeData } from "./flow-drag.js";
import {
  normalizeFlowControllerPortPlacement,
  type FlowControllerPortPlacement,
} from "./flow-controller-port-placement.js";
import {
  buildResolvedNodeData,
  cloneFlowPortParameterValues,
  createFlowDefinitionLookup,
  expandDefinitionPorts,
  formatFlowDefinitionTitle,
} from "./flow-node-definitions.js";
import {
  cloneFlowStudioPortNames,
  resolveFlowStudioResolvedPorts,
  type FlowStudioResolvedPort,
} from "./flow-node-ports.js";
import {
  cloneFlowStudioPortOrder,
  hasFlowStudioPortOrder,
  type FlowStudioPortOrder,
} from "./flow-port-order.js";

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
export const DEFAULT_FLOW_OUTPUT_ENCODING = "xml";

/**
 * Default flow-wide plain-fragment delimiter when a workspace lacks one.
 * Matches the engine-side `default_flow_plain_fragment_delimiter`.
 */
export const DEFAULT_FLOW_PLAIN_FRAGMENT_DELIMITER = "blank_line";

/**
 * Default flow-wide whitespace mode when a workspace lacks one. Matches the
 * engine-side `default_flow_whitespace_mode`.
 */
export const DEFAULT_FLOW_WHITESPACE_MODE = "trim";

const FALLBACK_NODE_GRID = {
  startX: 80,
  startY: 120,
  xStep: 320,
  yStep: 180,
  columns: 4,
} as const;
/**
 * This editor's namespace within a flow document's client-scoped layout.
 *
 * Layout is a map of writer namespace to opaque blob; the engine reads the key
 * and nothing below it. Versioning the key is what lets this editor's canvas
 * dialect change without an engine release, and what keeps a second client
 * rendering the same flow from writing an incompatible shape to the same place.
 */
export const FLOW_EDITOR_LAYOUT_KEY = "flow_builder_v1";

const FLOW_LAYOUT_COMPARE_PRECISION_DIGITS = 6;

export function buildDefaultFlowWorkspace(): FlowStudioWorkspaceState {
  return {
    baselineFlow: createBlankFlowDocument(),
    description: "",
    draftFlowKey: "",
    edges: [],
    nodes: [],
    outputEncoding: DEFAULT_FLOW_OUTPUT_ENCODING,
    plainFragmentDelimiter: DEFAULT_FLOW_PLAIN_FRAGMENT_DELIMITER,
    whitespaceMode: DEFAULT_FLOW_WHITESPACE_MODE,
    selectedFlowKey: "",
    selectedTarget: { kind: "none" },
    title: "",
  };
}

export function createBlankFlowDocument(): WireFlowDocument {
  return {
    version: FLOW_DOCUMENT_VERSION,
    execution: createFlowExecutionPolicy(),
    flow_key: "",
    title: "",
    description: "",
    output_encoding: DEFAULT_FLOW_OUTPUT_ENCODING,
    plain_fragment_delimiter: DEFAULT_FLOW_PLAIN_FRAGMENT_DELIMITER,
    whitespace_mode: DEFAULT_FLOW_WHITESPACE_MODE,
    nodes: [],
    edges: [],
    layout: undefined,
    metadata: null,
  };
}

export function normalizeFlowWorkspaceState(
  value: unknown,
): FlowStudioWorkspaceState {
  const candidate = isRecord(value)
    ? (value as Partial<FlowStudioWorkspaceState> & {
        selectedNodeId?: unknown;
      })
    : {};

  return {
    ...buildDefaultFlowWorkspace(),
    ...candidate,
    baselineFlow: isFlowDocument(candidate.baselineFlow)
      ? candidate.baselineFlow
      : createBlankFlowDocument(),
    description:
      typeof candidate.description === "string" ? candidate.description : "",
    draftFlowKey:
      typeof candidate.draftFlowKey === "string" ? candidate.draftFlowKey : "",
    edges: Array.isArray(candidate.edges)
      ? candidate.edges.map(normalizeCanvasEdge)
      : [],
    nodes: Array.isArray(candidate.nodes)
      ? candidate.nodes.map(normalizeCanvasNode)
      : [],
    outputEncoding:
      typeof candidate.outputEncoding === "string" &&
      candidate.outputEncoding !== ""
        ? candidate.outputEncoding
        : DEFAULT_FLOW_OUTPUT_ENCODING,
    plainFragmentDelimiter:
      typeof candidate.plainFragmentDelimiter === "string" &&
      candidate.plainFragmentDelimiter !== ""
        ? candidate.plainFragmentDelimiter
        : DEFAULT_FLOW_PLAIN_FRAGMENT_DELIMITER,
    whitespaceMode:
      typeof candidate.whitespaceMode === "string" &&
      candidate.whitespaceMode !== ""
        ? candidate.whitespaceMode
        : DEFAULT_FLOW_WHITESPACE_MODE,
    selectedFlowKey:
      typeof candidate.selectedFlowKey === "string"
        ? candidate.selectedFlowKey
        : "",
    selectedTarget: normalizeSelectionTarget(candidate),
    title: typeof candidate.title === "string" ? candidate.title : "",
  };
}

export function reconcileFlowWorkspaceDefinitions(
  workspace: FlowStudioWorkspaceState,
  definitionLookup: Record<string, WireFlowNodeDefinition>,
): FlowStudioWorkspaceState {
  let changed = false;
  const nodes = workspace.nodes.map((node) => {
    const definition = definitionLookup[node.data.definitionName];
    if (!definition) {
      return node;
    }

    const nextData = buildResolvedNodeData({
      controllerPortPlacement: node.data.controllerPortPlacement,
      definition,
      instanceName: node.data.instanceName,
      parameterValues: isRecord(node.data.parameterValues)
        ? node.data.parameterValues
        : {},
      portParameterValues: node.data.portParameterValues,
      portOrder: node.data.portOrder,
      portNames: node.data.portNames,
    });
    const nextClassName = `flow-studio-node flow-studio-node--${nextData.nodeClass}`;

    if (
      node.className === nextClassName &&
      stableJsonStringify(node.data) === stableJsonStringify(nextData)
    ) {
      return node;
    }

    changed = true;
    return {
      ...node,
      className: nextClassName,
      data: nextData,
    };
  });

  return changed
    ? {
        ...workspace,
        nodes,
      }
    : workspace;
}

function normalizeCanvasNode(
  node: Node<FlowStudioNodeData>,
): Node<FlowStudioNodeData> {
  const rawData = (node.data ?? {}) as Partial<FlowStudioNodeData>;
  const nodeClass =
    rawData.nodeClass === "source" ||
    rawData.nodeClass === "control" ||
    rawData.nodeClass === "hybrid" ||
    rawData.nodeClass === "instrument" ||
    rawData.nodeClass === "logic" ||
    rawData.nodeClass === "inline" ||
    rawData.nodeClass === "sink"
      ? rawData.nodeClass
      : "inline";

  return {
    ...node,
    className: `flow-studio-node flow-studio-node--${nodeClass}`,
    data: {
      actionPorts: Array.isArray(rawData.actionPorts)
        ? resolveFlowStudioResolvedPorts(
            rawData.actionPorts,
            "action",
            nodeClass,
          )
        : [],
      automationPorts: Array.isArray(rawData.automationPorts)
        ? resolveFlowStudioResolvedPorts(
            rawData.automationPorts,
            "automation",
            nodeClass,
          )
        : [],
      controllerPortPlacement: normalizeFlowControllerPortPlacement(
        rawData.controllerPortPlacement,
      ),
      definitionName:
        typeof rawData.definitionName === "string"
          ? rawData.definitionName
          : "",
      hasController: rawData.hasController === true,
      inputPorts: Array.isArray(rawData.inputPorts)
        ? resolveFlowStudioResolvedPorts(rawData.inputPorts, "input", nodeClass)
        : [],
      instanceName:
        typeof rawData.instanceName === "string" ? rawData.instanceName : "",
      longDescription:
        typeof rawData.longDescription === "string"
          ? rawData.longDescription
          : "",
      nodeClass,
      parameterValues: isRecord(rawData.parameterValues)
        ? rawData.parameterValues
        : {},
      portParameterValues: isFlowPortParameterValues(
        rawData.portParameterValues,
      )
        ? cloneFlowPortParameterValues(rawData.portParameterValues)
        : undefined,
      outputPorts: Array.isArray(rawData.outputPorts)
        ? resolveFlowStudioResolvedPorts(
            rawData.outputPorts,
            "output",
            nodeClass,
          )
        : [],
      portOrder: isFlowStudioPortOrder(rawData.portOrder)
        ? cloneFlowStudioPortOrder(rawData.portOrder)
        : undefined,
      portNames: isFlowStudioPortNames(rawData.portNames)
        ? cloneFlowStudioPortNames(rawData.portNames)
        : undefined,
      signalPorts: Array.isArray(rawData.signalPorts)
        ? resolveFlowStudioResolvedPorts(
            rawData.signalPorts,
            "signal",
            nodeClass,
          )
        : [],
      shortDescription:
        typeof rawData.shortDescription === "string"
          ? rawData.shortDescription
          : "",
    },
    position: {
      x:
        typeof node.position?.x === "number" && Number.isFinite(node.position.x)
          ? node.position.x
          : 0,
      y:
        typeof node.position?.y === "number" && Number.isFinite(node.position.y)
          ? node.position.y
          : 0,
    },
    type: node.type ?? "flowStudio",
  };
}

function normalizeCanvasEdge(edge: FlowStudioEdge): FlowStudioEdge {
  return {
    ...edge,
    data: {
      bridges: Array.isArray(edge.data?.bridges)
        ? edge.data.bridges
            .map(normalizeEdgeBridge)
            .filter((bridge): bridge is FlowStudioEdgeBridge => bridge !== null)
        : undefined,
      kind: edge.data?.kind === "signal" ? "signal" : "token",
      order: typeof edge.data?.order === "number" ? edge.data.order : 0,
      waypoints: Array.isArray(edge.data?.waypoints)
        ? edge.data.waypoints
            .map(normalizeEdgeWaypoint)
            .filter(
              (waypoint): waypoint is FlowStudioEdgeWaypoint =>
                waypoint !== null,
            )
        : undefined,
    },
    sourceHandle:
      typeof edge.sourceHandle === "string" ? edge.sourceHandle : null,
    targetHandle:
      typeof edge.targetHandle === "string" ? edge.targetHandle : null,
  };
}

function isFlowDocument(value: unknown): value is WireFlowDocument {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }

  const candidate = value as Partial<WireFlowDocument>;
  if (typeof candidate.version === "number") {
    requireSupportedFlowVersion(candidate as WireFlowDocument);
  }
  return (
    typeof candidate.version === "number" &&
    typeof candidate.flow_key === "string" &&
    typeof candidate.title === "string" &&
    Array.isArray(candidate.nodes) &&
    Array.isArray(candidate.edges)
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

export function buildFlowDocumentFromWorkspace(params: {
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
}): WireFlowDocument {
  if (params.baselineFlow) requireSupportedFlowVersion(params.baselineFlow);
  const title = params.title.trim();
  const nodes = [...params.nodes].sort((left, right) =>
    left.id.localeCompare(right.id),
  );
  const edges = [...params.edges].sort((left, right) =>
    left.id.localeCompare(right.id),
  );

  const execution = structuredClone(
    params.baselineFlow?.execution ?? createFlowExecutionPolicy(),
  );
  const sources = params.nodes
    .filter(node => node.data.nodeClass === "source" || node.data.nodeClass === "hybrid")
    .map(node => node.id);
  execution.source_order = execution.source_order.filter(id => sources.includes(id));
  for (const id of sources) {
    if (!execution.source_order.includes(id)) execution.source_order.push(id);
  }
  return {
    version: FLOW_DOCUMENT_VERSION,
    execution,
    flow_key: params.draftFlowKey,
    title,
    description: params.description.trim(),
    output_encoding: params.outputEncoding ?? DEFAULT_FLOW_OUTPUT_ENCODING,
    plain_fragment_delimiter:
      params.plainFragmentDelimiter ?? DEFAULT_FLOW_PLAIN_FRAGMENT_DELIMITER,
    whitespace_mode: params.whitespaceMode ?? DEFAULT_FLOW_WHITESPACE_MODE,
    nodes: nodes.map(buildWireFlowNodeFromCanvasNode),
    edges: edges.map((edge) => buildWireFlowEdgeFromCanvasEdge(
      edge, nodes, execution.limits.provider_queue,
    )),
    layout: {
      ...(isRecord(params.baselineFlow?.layout)
        ? params.baselineFlow.layout
        : {}),
      ...buildFlowLayoutFromCanvas(nodes, edges),
    },
    metadata: params.baselineFlow?.metadata ?? null,
  };
}

export function buildFlowValidationDocument(params: {
  workspace: Pick<
    FlowStudioWorkspaceState,
    "description" | "draftFlowKey" | "edges" | "nodes" | "title"
  > &
    Partial<
      Pick<
        FlowStudioWorkspaceState,
        | "baselineFlow"
        | "outputEncoding"
        | "plainFragmentDelimiter"
        | "whitespaceMode"
      >
    >;
}): WireFlowDocument {
  const document = buildFlowDocumentFromWorkspace({
    baselineFlow: params.workspace.baselineFlow,
    description: params.workspace.description,
    draftFlowKey: params.workspace.draftFlowKey,
    edges: params.workspace.edges,
    nodes: params.workspace.nodes,
    outputEncoding: params.workspace.outputEncoding,
    plainFragmentDelimiter: params.workspace.plainFragmentDelimiter,
    whitespaceMode: params.workspace.whitespaceMode,
    title: params.workspace.title,
  });

  return {
    ...document,
    flow_key: document.flow_key.trim() || "__draft_validation__",
    title: document.title.trim() || "Untitled flow",
  };
}

export function buildFlowSaveDocument(params: {
  titleOverride?: string;
  workspace: Pick<
    FlowStudioWorkspaceState,
    "description" | "draftFlowKey" | "edges" | "nodes" | "title"
  > &
    Partial<
      Pick<
        FlowStudioWorkspaceState,
        | "baselineFlow"
        | "outputEncoding"
        | "plainFragmentDelimiter"
        | "whitespaceMode"
      >
    >;
}): WireFlowDocument | null {
  const title = (params.titleOverride ?? params.workspace.title).trim();
  if (!title) {
    return null;
  }

  return {
    ...buildFlowDocumentFromWorkspace({
      baselineFlow: params.workspace.baselineFlow,
      description: params.workspace.description,
      draftFlowKey: params.workspace.draftFlowKey,
      edges: params.workspace.edges,
      nodes: params.workspace.nodes,
      outputEncoding: params.workspace.outputEncoding,
      plainFragmentDelimiter: params.workspace.plainFragmentDelimiter,
      whitespaceMode: params.workspace.whitespaceMode,
      title,
    }),
    flow_key: params.workspace.draftFlowKey.trim() || slugifyFlowKey(title),
    title,
  };
}

export function buildFlowWorkspaceFromDocument(params: {
  definitions: WireFlowNodeDefinition[];
  document: WireFlowDocument;
}): FlowStudioWorkspaceState {
  requireSupportedFlowVersion(params.document);
  const definitionLookup = createFlowDefinitionLookup(params.definitions);
  const nodeDocsById = Object.fromEntries(
    params.document.nodes.map((node) => [node.id, node]),
  );
  const layoutEdgeWaypoints = resolveLayoutEdgeWaypointMap(
    params.document.layout,
  );
  const mappedEdges = params.document.edges
    .map((edge) => {
      const layoutEdge = layoutEdgeWaypoints[edge.id];

      return mapWireEdgeToCanvasEdge(
        edge,
        nodeDocsById,
        definitionLookup,
        isRecord(layoutEdge) ? layoutEdge : undefined,
      );
    })
    .filter((edge): edge is FlowStudioEdge => edge !== null);
  const nodes = params.document.nodes
    .map((node, index) =>
      mapWireNodeToCanvasNode(
        node,
        definitionLookup[node.definition_name],
        resolveLayoutNodeState(params.document.layout, node.id, index),
      ),
    )
    .filter((node): node is Node<FlowStudioNodeData> => node !== null);

  return {
    baselineFlow: cloneFlowDocument(params.document),
    description: params.document.description?.trim() ?? "",
    draftFlowKey: params.document.flow_key,
    edges: mappedEdges,
    nodes,
    outputEncoding:
      params.document.output_encoding?.trim() || DEFAULT_FLOW_OUTPUT_ENCODING,
    plainFragmentDelimiter:
      params.document.plain_fragment_delimiter?.trim() ||
      DEFAULT_FLOW_PLAIN_FRAGMENT_DELIMITER,
    whitespaceMode:
      params.document.whitespace_mode?.trim() || DEFAULT_FLOW_WHITESPACE_MODE,
    selectedFlowKey: params.document.flow_key,
    selectedTarget: { kind: "none" },
    title: params.document.title ?? "",
  };
}

export function areFlowDocumentsEqual(
  left: WireFlowDocument,
  right: WireFlowDocument,
): boolean {
  return (
    stableJsonStringify(normalizeFlowDocumentForCompare(left)) ===
    stableJsonStringify(normalizeFlowDocumentForCompare(right))
  );
}

export function slugifyFlowKey(title: string): string {
  const slug = title
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");

  return slug || "untitled-flow";
}

export function findNextFlowNodeIndex(
  nodes: Array<Node<FlowStudioNodeData>>,
): number {
  return (
    nodes.reduce((maxIndex, node) => {
      const match = node.id.match(/-(\d+)$/);
      const value = match ? Number.parseInt(match[1], 10) : 0;
      return Number.isFinite(value) ? Math.max(maxIndex, value) : maxIndex;
    }, 0) + 1
  );
}

export function removeNodeFromWorkspace(
  workspace: FlowStudioWorkspaceState,
  nodeId: string,
): FlowStudioWorkspaceState {
  return {
    ...workspace,
    edges: workspace.edges.filter(
      (edge) => edge.source !== nodeId && edge.target !== nodeId,
    ),
    nodes: workspace.nodes.filter((node) => node.id !== nodeId),
    selectedTarget:
      (workspace.selectedTarget.kind === "node" ||
        workspace.selectedTarget.kind === "port") &&
      workspace.selectedTarget.nodeId === nodeId
        ? { kind: "none" }
        : workspace.selectedTarget,
  };
}

export function renameNodeInstanceInWorkspace(
  workspace: FlowStudioWorkspaceState,
  nodeId: string,
  instanceName: string,
): FlowStudioWorkspaceState {
  const trimmedInstanceName = instanceName.trim();
  if (!trimmedInstanceName) {
    return workspace;
  }

  return {
    ...workspace,
    nodes: workspace.nodes.map((node) =>
      node.id === nodeId
        ? {
            ...node,
            data: {
              ...node.data,
              instanceName: trimmedInstanceName,
            },
          }
        : node,
    ),
  };
}

function buildWireFlowNodeFromCanvasNode(
  node: Node<FlowStudioNodeData>,
): WireFlowNode {
  return {
    definition_name: node.data.definitionName,
    id: node.id,
    instance_name: node.data.instanceName,
    parameter_values: sortJsonObject(node.data.parameterValues),
    port_parameter_values: canonicalizeFlowPortParameterValues(
      node.data.portParameterValues,
    ),
    port_order: canonicalizeFlowStudioPortOrder(node.data.portOrder),
    port_names: canonicalizeFlowStudioPortNames(node.data.portNames),
  };
}

function buildWireFlowEdgeFromCanvasEdge(
  edge: FlowStudioEdge,
  nodes: Array<Node<FlowStudioNodeData>>,
  defaultQueue: NonNullable<WireFlowEdge["queue"]>,
): WireFlowEdge {
  const sourceNode = nodes.find((node) => node.id === edge.source);
  const targetNode = nodes.find((node) => node.id === edge.target);
  const edgeKind = edge.data?.kind === "signal" ? "signal" : "token";
  const sourcePorts =
    edgeKind === "signal"
      ? resolveFlowStudioResolvedPorts(
          sourceNode?.data.signalPorts ?? [],
          "signal",
        )
      : resolveFlowStudioResolvedPorts(
          sourceNode?.data.outputPorts ?? [],
          "output",
        );

  // Token edges may target either an input port or an automation port on the
  // host node. Pick the list based on the saved handle id prefix.
  const targetSide: "action" | "automation" | "input" =
    edgeKind === "signal"
      ? "action"
      : edge.targetHandle?.startsWith("automation-")
        ? "automation"
        : "input";
  const targetPorts =
    targetSide === "action"
      ? resolveFlowStudioResolvedPorts(
          targetNode?.data.actionPorts ?? [],
          "action",
        )
      : targetSide === "automation"
        ? resolveFlowStudioResolvedPorts(
            targetNode?.data.automationPorts ?? [],
            "automation",
          )
        : resolveFlowStudioResolvedPorts(
            targetNode?.data.inputPorts ?? [],
            "input",
          );

  const sourcePort = resolveHandlePortId(
    sourcePorts, edge.sourceHandle, edgeKind === "signal" ? "signal" : "output",
  );
  const streaming = edgeKind === "token" && sourcePorts.find(port => port.id === sourcePort)?.mode === "stream";
  return {
    id: edge.id,
    kind: edgeKind,
    ...(streaming ? { queue: structuredClone(edge.data?.queue ?? defaultQueue) } : {}),
    order: typeof edge.data?.order === "number" ? edge.data.order : 0,
    source_node_id: edge.source,
    source_port: sourcePort,
    target_node_id: edge.target,
    target_port: resolveHandlePortId(
      targetPorts,
      edge.targetHandle,
      targetSide,
    ),
  };
}

function mapWireNodeToCanvasNode(
  node: WireFlowNode,
  definition: WireFlowNodeDefinition | undefined,
  layout: {
    controllerPortPlacement: FlowControllerPortPlacement;
    position: { x: number; y: number };
  },
): Node<FlowStudioNodeData> | null {
  if (!definition) {
    return null;
  }

  return {
    id: node.id,
    className: `flow-studio-node flow-studio-node--${definition.kind}`,
    data: buildResolvedNodeData({
      controllerPortPlacement: layout.controllerPortPlacement,
      definition,
      instanceName: node.instance_name,
      parameterValues: node.parameter_values ?? {},
      portParameterValues: node.port_parameter_values,
      portOrder: node.port_order,
      portNames: node.port_names,
    }),
    position: layout.position,
    type: "flowStudio",
  };
}

function mapWireEdgeToCanvasEdge(
  edge: WireFlowEdge,
  nodesById: Record<string, WireFlowNode>,
  definitionLookup: Record<string, WireFlowNodeDefinition>,
  layoutEdge: unknown,
): FlowStudioEdge | null {
  const sourceNode = nodesById[edge.source_node_id];
  const targetNode = nodesById[edge.target_node_id];

  if (!sourceNode || !targetNode) {
    return null;
  }

  const sourceDefinition = definitionLookup[sourceNode.definition_name];
  const targetDefinition = definitionLookup[targetNode.definition_name];
  if (!sourceDefinition || !targetDefinition) {
    return null;
  }

  const resolvedTarget = buildResolvedNodeData({
    definition: targetDefinition,
    instanceName: targetNode.instance_name,
    parameterValues: targetNode.parameter_values ?? {},
    portOrder: targetNode.port_order,
    portNames: targetNode.port_names,
  });
  const sourcePorts =
    edge.kind === "signal"
      ? buildResolvedNodeData({
          definition: sourceDefinition,
          instanceName: sourceNode.instance_name,
          parameterValues: sourceNode.parameter_values ?? {},
          portOrder: sourceNode.port_order,
          portNames: sourceNode.port_names,
        }).signalPorts
      : buildResolvedNodeData({
          definition: sourceDefinition,
          instanceName: sourceNode.instance_name,
          parameterValues: sourceNode.parameter_values ?? {},
          portOrder: sourceNode.port_order,
          portNames: sourceNode.port_names,
        }).outputPorts;
  const sourceIndex = resolveFlowStudioResolvedPorts(
    sourcePorts,
    edge.kind === "signal" ? "signal" : "output",
  ).findIndex((port) => port.id === edge.source_port);

  // Token edges may target an input port or an automation port on the host
  // node; pick whichever list contains the saved target_port name.
  let targetIndex = -1;
  let targetSide: "action" | "automation" | "input";
  if (edge.kind === "signal") {
    targetSide = "action";
    targetIndex = resolveFlowStudioResolvedPorts(
      resolvedTarget.actionPorts,
      "action",
    ).findIndex((port) => port.id === edge.target_port);
  } else {
    const inputIndex = resolveFlowStudioResolvedPorts(
      resolvedTarget.inputPorts,
      "input",
    ).findIndex((port) => port.id === edge.target_port);
    if (inputIndex >= 0) {
      targetSide = "input";
      targetIndex = inputIndex;
    } else {
      const automationIndex = resolveFlowStudioResolvedPorts(
        resolvedTarget.automationPorts,
        "automation",
      ).findIndex((port) => port.id === edge.target_port);
      targetSide = "automation";
      targetIndex = automationIndex;
    }
  }

  if (sourceIndex < 0 || targetIndex < 0) {
    return null;
  }

  return {
    data: {
      bridges: isRecord(layoutEdge)
        ? resolveLayoutEdgeBridges(layoutEdge.bridges)
        : undefined,
      kind: edge.kind,
      order: edge.order,
      ...(edge.queue ? { queue: structuredClone(edge.queue) } : {}),
      waypoints: isRecord(layoutEdge)
        ? resolveLayoutEdgeWaypoints(layoutEdge.waypoints)
        : undefined,
    },
    id: edge.id,
    source: edge.source_node_id,
    sourceHandle: `${edge.kind === "signal" ? "signal" : "output"}-${sourceIndex}`,
    target: edge.target_node_id,
    targetHandle: `${targetSide}-${targetIndex}`,
  };
}

function resolveHandlePortId(
  ports: readonly FlowStudioResolvedPort[],
  handleId: string | null | undefined,
  side: "action" | "automation" | "input" | "output" | "signal",
): string {
  const fallback = ports[0]?.id ?? side;
  if (!handleId?.startsWith(`${side}-`)) {
    return fallback;
  }

  const index = Number.parseInt(handleId.slice(side.length + 1), 10);
  return Number.isFinite(index) && ports[index] ? ports[index].id : fallback;
}

function resolveLayoutNodeState(
  layout: unknown,
  nodeId: string,
  index: number,
): {
  controllerPortPlacement: FlowControllerPortPlacement;
  position: { x: number; y: number };
} {
  const canvas = resolveFlowEditorCanvasLayout(layout);
  const nodes = isRecord(canvas?.nodes) ? canvas.nodes : null;
  const nodeLayout = isRecord(nodes?.[nodeId]) ? nodes[nodeId] : null;
  const position = isRecord(nodeLayout?.position) ? nodeLayout.position : null;
  const controllerPortPlacement = normalizeFlowControllerPortPlacement(
    nodeLayout?.controllerPortPlacement,
  );
  const x =
    typeof position?.x === "number" && Number.isFinite(position.x)
      ? position.x
      : null;
  const y =
    typeof position?.y === "number" && Number.isFinite(position.y)
      ? position.y
      : null;

  if (x !== null && y !== null) {
    return {
      controllerPortPlacement,
      position: { x, y },
    };
  }

  const column = index % FALLBACK_NODE_GRID.columns;
  const row = Math.floor(index / FALLBACK_NODE_GRID.columns);
  return {
    controllerPortPlacement,
    position: {
      x: FALLBACK_NODE_GRID.startX + column * FALLBACK_NODE_GRID.xStep,
      y: FALLBACK_NODE_GRID.startY + row * FALLBACK_NODE_GRID.yStep,
    },
  };
}

function buildFlowLayoutFromCanvas(
  nodes: Array<Node<FlowStudioNodeData>>,
  edges: readonly FlowStudioEdge[],
): Record<string, unknown> {
  const edgeLayouts = Object.fromEntries(
    edges
      .filter(
        (edge) =>
          (Array.isArray(edge.data?.bridges) && edge.data.bridges.length > 0) ||
          (Array.isArray(edge.data?.waypoints) &&
            edge.data.waypoints.length > 0),
      )
      .map((edge) => [
        edge.id,
        {
          ...(edge.data?.bridges?.length
            ? {
                bridges: edge.data.bridges.map((bridge) => ({
                  gap: bridge.gap,
                  segmentIndex: bridge.segmentIndex,
                  t: bridge.t,
                })),
              }
            : {}),
          waypoints: edge.data?.waypoints?.map((waypoint) => ({
            inHandle: waypoint.inHandle,
            outHandle: waypoint.outHandle,
            position: waypoint.position,
          })),
        },
      ]),
  );

  return {
    [FLOW_EDITOR_LAYOUT_KEY]: {
      canvas: {
        ...(Object.keys(edgeLayouts).length > 0 ? { edges: edgeLayouts } : {}),
        nodes: Object.fromEntries(
          nodes.map((node) => [
            node.id,
            {
              ...(node.data.controllerPortPlacement === "swapped"
                ? { controllerPortPlacement: node.data.controllerPortPlacement }
                : {}),
              position: {
                x: node.position.x,
                y: node.position.y,
              },
            },
          ]),
        ),
      },
    },
  };
}

function canonicalizeFlowStudioPortNames(
  portNames: WireFlowNode["port_names"] | null | undefined,
): WireFlowNode["port_names"] | undefined {
  const cloned = cloneFlowStudioPortNames(portNames);
  if (!cloned) {
    return undefined;
  }

  const canonical = {
    action: sortOptionalStringMap(cloned.action),
    input: sortOptionalStringMap(cloned.input),
    output: sortOptionalStringMap(cloned.output),
    signal: sortOptionalStringMap(cloned.signal),
  } satisfies NonNullable<WireFlowNode["port_names"]>;

  return Object.values(canonical).some((value) => value !== undefined)
    ? canonical
    : undefined;
}

function canonicalizeFlowStudioPortOrder(
  portOrder: FlowStudioPortOrder | null | undefined,
): WireFlowNode["port_order"] | undefined {
  const cloned = cloneFlowStudioPortOrder(portOrder);
  if (!cloned) {
    return undefined;
  }

  const canonical = {
    action: sortOptionalStringArray(cloned.action),
    input: sortOptionalStringArray(cloned.input),
    output: sortOptionalStringArray(cloned.output),
    signal: sortOptionalStringArray(cloned.signal),
  } satisfies NonNullable<WireFlowNode["port_order"]>;

  return hasFlowStudioPortOrder(canonical) ? canonical : undefined;
}

function canonicalizeFlowPortParameterValues(
  portParameterValues: WireFlowNode["port_parameter_values"] | null | undefined,
): WireFlowNode["port_parameter_values"] | undefined {
  const cloned = cloneFlowPortParameterValues(portParameterValues);
  if (!cloned) {
    return undefined;
  }

  const canonicalSide = (
    side: Record<string, Record<string, unknown>> | undefined,
  ): Record<string, Record<string, unknown>> | undefined => {
    if (!side || Object.keys(side).length === 0) {
      return undefined;
    }
    const entries = Object.entries(side)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([portId, values]) => [portId, sortJsonObject(values)] as const)
      .filter(([, values]) => Object.keys(values).length > 0);
    return entries.length > 0 ? Object.fromEntries(entries) : undefined;
  };

  const canonical = {
    input: canonicalSide(cloned.input),
    output: canonicalSide(cloned.output),
  } satisfies NonNullable<WireFlowNode["port_parameter_values"]>;

  return canonical.input || canonical.output ? canonical : undefined;
}

function sortOptionalStringMap(
  value: Record<string, string> | null | undefined,
): Record<string, string> | undefined {
  if (!value || Object.keys(value).length === 0) {
    return undefined;
  }

  return Object.fromEntries(
    Object.entries(value).sort(([left], [right]) => left.localeCompare(right)),
  );
}

function sortOptionalStringArray(
  value: string[] | null | undefined,
): string[] | undefined {
  return value && value.length > 0 ? [...value] : undefined;
}

function normalizeEdgeWaypoint(value: unknown): FlowStudioEdgeWaypoint | null {
  const waypoint = isRecord(value) ? value : null;
  const position = normalizeWaypointPoint(waypoint?.position);
  const inHandle = normalizeWaypointPoint(waypoint?.inHandle);
  const outHandle = normalizeWaypointPoint(waypoint?.outHandle);

  if (!position || !inHandle || !outHandle) {
    return null;
  }

  return {
    inHandle,
    outHandle,
    position,
  };
}

function normalizeEdgeBridge(value: unknown): FlowStudioEdgeBridge | null {
  const bridge = isRecord(value) ? value : null;
  const gap =
    typeof bridge?.gap === "number" && Number.isFinite(bridge.gap)
      ? bridge.gap
      : null;
  const segmentIndex =
    typeof bridge?.segmentIndex === "number" &&
    Number.isFinite(bridge.segmentIndex)
      ? bridge.segmentIndex
      : null;
  const t =
    typeof bridge?.t === "number" && Number.isFinite(bridge.t)
      ? bridge.t
      : null;

  if (gap === null || segmentIndex === null || t === null) {
    return null;
  }

  return {
    gap,
    segmentIndex,
    t,
  };
}

function normalizeWaypointPoint(
  value: unknown,
): FlowStudioEdgeWaypoint["position"] | null {
  const point = isRecord(value) ? value : null;
  const x =
    typeof point?.x === "number" && Number.isFinite(point.x) ? point.x : null;
  const y =
    typeof point?.y === "number" && Number.isFinite(point.y) ? point.y : null;

  return x === null || y === null ? null : { x, y };
}

function resolveLayoutEdgeWaypointMap(
  layout: unknown,
): Record<string, unknown> {
  const canvas = resolveFlowEditorCanvasLayout(layout);
  return isRecord(canvas?.edges) ? canvas.edges : {};
}

/**
 * Reads this editor's canvas state out of the document's client-scoped layout.
 *
 * Layout is a map of writer namespace to opaque blob, so a second client
 * rendering the same flow keeps its geometry under its own key. The engine
 * merges by namespace on save, so writing only this key preserves the others.
 */
function resolveFlowEditorCanvasLayout(
  layout: unknown,
): Record<string, unknown> | null {
  const envelope = isRecord(layout) ? layout[FLOW_EDITOR_LAYOUT_KEY] : null;
  const canvas = isRecord(envelope) ? envelope.canvas : null;
  return isRecord(canvas) ? canvas : null;
}

function resolveLayoutEdgeWaypoints(
  value: unknown,
): FlowStudioEdgeWaypoint[] | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }

  const waypoints = value
    .map(normalizeEdgeWaypoint)
    .filter(
      (waypoint): waypoint is FlowStudioEdgeWaypoint => waypoint !== null,
    );

  return waypoints.length > 0 ? waypoints : undefined;
}

function resolveLayoutEdgeBridges(
  value: unknown,
): FlowStudioEdgeBridge[] | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }

  const bridges = value
    .map(normalizeEdgeBridge)
    .filter((bridge): bridge is FlowStudioEdgeBridge => bridge !== null);

  return bridges.length > 0 ? bridges : undefined;
}

function isFlowStudioPortNames(
  value: unknown,
): value is NonNullable<WireFlowNode["port_names"]> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }

  return ["action", "input", "output", "signal"].every((side) => {
    const candidate = (value as Record<string, unknown>)[side];
    if (candidate === undefined) {
      return true;
    }

    return (
      isRecord(candidate) &&
      Object.values(candidate).every((alias) => typeof alias === "string")
    );
  });
}

function isFlowStudioPortOrder(value: unknown): value is FlowStudioPortOrder {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }

  return ["action", "input", "output", "signal"].every((side) => {
    const candidate = (value as Record<string, unknown>)[side];
    return (
      candidate === undefined ||
      (Array.isArray(candidate) &&
        candidate.every((portId) => typeof portId === "string"))
    );
  });
}

function isFlowPortParameterValues(
  value: unknown,
): value is NonNullable<WireFlowNode["port_parameter_values"]> {
  if (!isRecord(value)) {
    return false;
  }

  return ["input", "output"].every((side) => {
    const candidate = value[side];
    if (candidate === undefined) {
      return true;
    }
    return (
      isRecord(candidate) &&
      Object.values(candidate).every((portValues) => isRecord(portValues))
    );
  });
}

function normalizeSelectionTarget(value: {
  selectedNodeId?: unknown;
  selectedTarget?: unknown;
}): FlowStudioSelectionTarget {
  const selectedTarget = value.selectedTarget;
  if (
    selectedTarget &&
    typeof selectedTarget === "object" &&
    !Array.isArray(selectedTarget) &&
    "kind" in selectedTarget
  ) {
    const target = selectedTarget as Partial<FlowStudioSelectionTarget>;
    if (target.kind === "flow") {
      return { kind: "flow" };
    }
    if (target.kind === "node" && typeof target.nodeId === "string") {
      return { kind: "node", nodeId: target.nodeId };
    }
    if (
      target.kind === "port" &&
      typeof target.nodeId === "string" &&
      typeof target.portId === "string" &&
      (target.side === "action" ||
        target.side === "automation" ||
        target.side === "input" ||
        target.side === "output" ||
        target.side === "signal")
    ) {
      return {
        kind: "port",
        nodeId: target.nodeId,
        portId: target.portId,
        side: target.side,
      };
    }
  }

  const legacySelectedNodeId = value.selectedNodeId;
  return typeof legacySelectedNodeId === "string" && legacySelectedNodeId
    ? { kind: "node", nodeId: legacySelectedNodeId }
    : { kind: "none" };
}

function cloneFlowDocument(document: WireFlowDocument): WireFlowDocument {
  return JSON.parse(JSON.stringify(document)) as WireFlowDocument;
}

function normalizeFlowDocumentForCompare(
  document: WireFlowDocument,
): WireFlowDocument {
  return {
    ...document,
    description: document.description ?? "",
    edges: [...document.edges].sort((left, right) =>
      left.id.localeCompare(right.id),
    ),
    layout: normalizeFlowLayoutForCompare(document.layout),
    metadata: normalizeJsonValue(document.metadata ?? null),
    whitespace_mode:
      document.whitespace_mode?.trim() || DEFAULT_FLOW_WHITESPACE_MODE,
    nodes: [...document.nodes]
      .map((node) => ({
        ...node,
        parameter_values: sortJsonObject(node.parameter_values),
        port_parameter_values: canonicalizeFlowPortParameterValues(
          node.port_parameter_values,
        ),
        port_names: canonicalizeFlowStudioPortNames(node.port_names),
        port_order: canonicalizeFlowStudioPortOrder(node.port_order),
      }))
      .sort((left, right) => left.id.localeCompare(right.id)),
  };
}

function sortJsonObject(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return {};
  }

  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, nested]) => [key, normalizeJsonValue(nested)]),
  );
}

/**
 * Reduces a layout to what a dirty comparison should notice.
 *
 * Only this editor's namespace is compared: another client moving its own
 * canvas is not an edit to this one, and the engine preserves that namespace
 * across our saves regardless.
 */
function normalizeFlowLayoutForCompare(
  layout: unknown,
): Record<string, unknown> | undefined {
  const normalizedLayout = normalizeFlowLayoutJsonValue(layout ?? null);
  if (!isRecord(normalizedLayout)) {
    return undefined;
  }

  const envelope = normalizedLayout[FLOW_EDITOR_LAYOUT_KEY];
  if (!isRecord(envelope)) {
    return undefined;
  }

  const canvas = isRecord(envelope.canvas) ? envelope.canvas : null;
  const nodes = isRecord(canvas?.nodes) ? canvas.nodes : null;
  if (!nodes || Object.keys(nodes).length === 0) {
    return undefined;
  }

  return { [FLOW_EDITOR_LAYOUT_KEY]: envelope };
}

function normalizeFlowLayoutJsonValue(value: unknown): unknown {
  if (typeof value === "number" && Number.isFinite(value)) {
    return Number(value.toFixed(FLOW_LAYOUT_COMPARE_PRECISION_DIGITS));
  }

  if (Array.isArray(value)) {
    return value.map(normalizeFlowLayoutJsonValue);
  }

  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, nested]) => [key, normalizeFlowLayoutJsonValue(nested)]),
    );
  }

  return value;
}

function normalizeJsonValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(normalizeJsonValue);
  }

  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, nested]) => [key, normalizeJsonValue(nested)]),
    );
  }

  return value;
}

function stableJsonStringify(value: unknown): string {
  return JSON.stringify(normalizeJsonValue(value));
}
