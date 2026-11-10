/** Copyright (c) Scott A Dixon */
import React from "react";
import {
  applyFlowEdgeMarking,
  type FlowActivityRecord,
  type FlowEdgeMarkingState,
} from "../core/flow-edge-marking.js";

/** Follows a subscription's records and returns the edge marking they describe. */
export function useFlowEdgeMarking(
  subscribe:
    | ((listener: (record: FlowActivityRecord) => void) => () => void)
    | null,
): FlowEdgeMarkingState {
  const [marking, setMarking] = React.useState<FlowEdgeMarkingState>({});
  React.useEffect(() => {
    setMarking({});
    if (!subscribe) return;
    return subscribe((record) =>
      setMarking((current) => applyFlowEdgeMarking(current, record)),
    );
  }, [subscribe]);
  return marking;
}
