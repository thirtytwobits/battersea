import React from "react";
import { createRoot } from "react-dom/client";
import {
  FlowEditor,
  type FlowInteractionPorts,
  type FlowLifecycleHost,
  type FlowParameterRenderer,
} from "@battersea/editor";
import { ports } from "./ports";
import "@xyflow/react/dist/style.css";
import "./style.css";
const notify: FlowInteractionPorts["notify"] = (message) => {
  const output = document.getElementById("notice");
  if (output) output.textContent = message.message;
};
const lifecycle: FlowLifecycleHost = {
  confirm: async (options) => window.confirm(options.message),
  confirmDiscard: async (options) => window.confirm(options.message),
  registerDirty: () => {},
  unregisterDirty: () => {},
  notify,
  deleteConfirmation: (title) => `Delete ${title}?`,
};
const interaction: FlowInteractionPorts = {
  notify,
  isKeyboardBlocked: (event) =>
    event.target instanceof HTMLElement &&
    event.target.closest("input,textarea,select,[contenteditable]") !== null,
  resolveDragGrabOffset: () => null,
  resolveDragReleasePoint: () => null,
};
const renderText: FlowParameterRenderer = ({ parameter, value, onChange }) =>
  parameter.name === "text" ? (
    <label>
      Text
      <textarea
        aria-label="Text"
        data-parameter-renderer="application"
        value={typeof value === "string" ? value : ""}
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  ) : undefined;
createRoot(document.getElementById("root")!).render(
  <>
    <h1>Battersea flow editor</h1>
    <output id="notice" aria-live="polite" />
    <FlowEditor
      ports={ports}
      lifecycle={lifecycle}
      interaction={interaction}
      parameterRenderers={[renderText]}
    />
  </>,
);
