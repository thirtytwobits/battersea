/**
 * Copyright (c) Scott A Dixon
 *
 * Declares shared handle descriptors and structural connection helpers for authoring graphs.
 */
import React from "react";
import { Handle, Position } from "@xyflow/react";

export type AuthoringGraphHandleSide = "top" | "right" | "bottom" | "left";

export type AuthoringGraphHandleDirection = "source" | "target";

export interface AuthoringGraphHandleDescriptor<
  TFamily extends string = string,
> {
  anchorOffset?: number;
  data?: Record<string, unknown>;
  direction: AuthoringGraphHandleDirection;
  family: TFamily;
  handleId: string;
  label?: string;
  orderIndex?: number;
  side: AuthoringGraphHandleSide;
}

export interface AuthoringGraphResolvedConnection<
  TFamily extends string = string,
> {
  family: TFamily;
  source: string;
  sourceHandleId: string;
  sourceHandle: AuthoringGraphHandleDescriptor<TFamily>;
  target: string;
  targetHandleId: string;
  targetHandle: AuthoringGraphHandleDescriptor<TFamily>;
}

export type AuthoringGraphConnectionInspectionFailure =
  | "family_mismatch"
  | "missing_endpoints"
  | "missing_source_handle"
  | "missing_target_handle"
  | "self_loop"
  | "source_direction_mismatch"
  | "target_direction_mismatch";

export type AuthoringGraphConnectionInspection<
  TFamily extends string = string,
> =
  | {
      connection: AuthoringGraphResolvedConnection<TFamily>;
      valid: true;
    }
  | {
      reason: AuthoringGraphConnectionInspectionFailure;
      valid: false;
    };

export interface AuthoringGraphHandleIndex<TFamily extends string = string> {
  byDirection: ReadonlyMap<
    AuthoringGraphHandleDirection,
    readonly AuthoringGraphHandleDescriptor<TFamily>[]
  >;
  byFamily: ReadonlyMap<
    TFamily,
    readonly AuthoringGraphHandleDescriptor<TFamily>[]
  >;
  byId: ReadonlyMap<string, AuthoringGraphHandleDescriptor<TFamily>>;
  bySide: ReadonlyMap<
    AuthoringGraphHandleSide,
    readonly AuthoringGraphHandleDescriptor<TFamily>[]
  >;
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

export interface InspectAuthoringGraphConnectionOptions<
  TFamily extends string = string,
> {
  allowSelfLoops?: boolean;
  connection: AuthoringGraphConnectionLike;
  defaultSourceHandleId?: string;
  defaultTargetHandleId?: string;
  sourceHandles: readonly AuthoringGraphHandleDescriptor<TFamily>[];
  targetHandles: readonly AuthoringGraphHandleDescriptor<TFamily>[];
}

export function indexAuthoringGraphHandles<TFamily extends string = string>(
  descriptors: readonly AuthoringGraphHandleDescriptor<TFamily>[],
): AuthoringGraphHandleIndex<TFamily> {
  const byId = new Map<string, AuthoringGraphHandleDescriptor<TFamily>>();
  const byFamily = new Map<
    TFamily,
    AuthoringGraphHandleDescriptor<TFamily>[]
  >();
  const bySide = new Map<
    AuthoringGraphHandleSide,
    AuthoringGraphHandleDescriptor<TFamily>[]
  >();
  const byDirection = new Map<
    AuthoringGraphHandleDirection,
    AuthoringGraphHandleDescriptor<TFamily>[]
  >();

  for (const descriptor of descriptors) {
    byId.set(descriptor.handleId, descriptor);
    pushHandleIndexEntry(byFamily, descriptor.family, descriptor);
    pushHandleIndexEntry(bySide, descriptor.side, descriptor);
    pushHandleIndexEntry(byDirection, descriptor.direction, descriptor);
  }

  return {
    byDirection,
    byFamily,
    byId,
    bySide,
  };
}

export function inspectAuthoringGraphConnection<
  TFamily extends string = string,
>(
  options: InspectAuthoringGraphConnectionOptions<TFamily>,
): AuthoringGraphConnectionInspection<TFamily> {
  const source = options.connection.source ?? null;
  const target = options.connection.target ?? null;
  if (!source || !target) {
    return { reason: "missing_endpoints", valid: false };
  }

  if (options.allowSelfLoops === false && source === target) {
    return { reason: "self_loop", valid: false };
  }

  const sourceHandleId =
    options.connection.sourceHandle ?? options.defaultSourceHandleId ?? null;
  if (!sourceHandleId) {
    return { reason: "missing_source_handle", valid: false };
  }

  const targetHandleId =
    options.connection.targetHandle ?? options.defaultTargetHandleId ?? null;
  if (!targetHandleId) {
    return { reason: "missing_target_handle", valid: false };
  }

  const sourceIndex = indexAuthoringGraphHandles(options.sourceHandles);
  const targetIndex = indexAuthoringGraphHandles(options.targetHandles);
  const sourceHandle = sourceIndex.byId.get(sourceHandleId);
  if (!sourceHandle) {
    return { reason: "missing_source_handle", valid: false };
  }

  const targetHandle = targetIndex.byId.get(targetHandleId);
  if (!targetHandle) {
    return { reason: "missing_target_handle", valid: false };
  }

  if (sourceHandle.direction !== "source") {
    return { reason: "source_direction_mismatch", valid: false };
  }

  if (targetHandle.direction !== "target") {
    return { reason: "target_direction_mismatch", valid: false };
  }

  if (sourceHandle.family !== targetHandle.family) {
    return { reason: "family_mismatch", valid: false };
  }

  return {
    connection: {
      family: sourceHandle.family,
      source,
      sourceHandle,
      sourceHandleId,
      target,
      targetHandle,
      targetHandleId,
    },
    valid: true,
  };
}

export function resolveAuthoringGraphConnection<
  TFamily extends string = string,
>(
  options: InspectAuthoringGraphConnectionOptions<TFamily>,
): AuthoringGraphResolvedConnection<TFamily> | null {
  const inspection = inspectAuthoringGraphConnection(options);
  return inspection.valid ? inspection.connection : null;
}

export function getAuthoringGraphHandlePosition(
  side: AuthoringGraphHandleSide,
): Position {
  if (side === "top") {
    return Position.Top;
  }
  if (side === "right") {
    return Position.Right;
  }
  if (side === "bottom") {
    return Position.Bottom;
  }

  return Position.Left;
}

export function AuthoringGraphHandle<TFamily extends string = string>({
  className,
  descriptor,
  isConnectable = true,
  onClick,
  style,
  title,
  ...dataAttributes
}: AuthoringGraphHandleProps<TFamily>): React.JSX.Element {
  const resolvedClassName = ["authoring-graph-handle", className]
    .filter(Boolean)
    .join(" ");

  return (
    <Handle
      className={resolvedClassName}
      data-handle-direction={descriptor.direction}
      data-handle-family={descriptor.family}
      data-handle-side={descriptor.side}
      id={descriptor.handleId}
      isConnectable={isConnectable}
      onClick={onClick}
      {...dataAttributes}
      position={getAuthoringGraphHandlePosition(descriptor.side)}
      style={style}
      title={title ?? descriptor.label}
      type={descriptor.direction}
    />
  );
}

function pushHandleIndexEntry<TKey, TValue>(
  map: Map<TKey, TValue[]>,
  key: TKey,
  value: TValue,
): void {
  const bucket = map.get(key);
  if (bucket) {
    bucket.push(value);
    return;
  }

  map.set(key, [value]);
}
