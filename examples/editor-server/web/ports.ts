import {applyRuntimeUpdate,RuntimeResyncRequired,type Runtime} from "@battersea/flow";
import type { FlowEditorPorts, FlowDiagnostic } from "@battersea/editor";
async function request<T>(
  path: string,
  method = "GET",
  body?: unknown,
): Promise<T> {
  const response = await fetch(`/api/${path}`, {
    method,
    headers: body === undefined ? {} : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) throw new Error(await response.text());
  return response.json() as Promise<T>;
}
export const ports: FlowEditorPorts = {
  documents: {
    list: () => request("flows"),
    read: (key) => request(`flows/${encodeURIComponent(key)}`),
    save: (document) => request("flows", "PUT", document),
    clone: (data) => request("flows/clone", "POST", data),
    delete: ({ flow_key }) =>
      request(`flows/${encodeURIComponent(flow_key)}`, "DELETE"),
  },
  catalogue: { read: () => request("catalogue") },
  validation: { validate: (document) => request("validate", "POST", document) },
  activation: {
    activate: (data) => request("activations", "POST", data),
    cancel: (id) => request(`activations/${encodeURIComponent(id)}`, "DELETE"),
    subscribe: (id, listener) => {
      let stopped = false;
      let timer: ReturnType<typeof setTimeout>;
      let sequence = 0;
      let snapshot:Runtime.Snapshot | null = null;
      const controller = new AbortController();
      const poll = async () => {
        try {
          const cursor = snapshot?.cursor;
          const query = cursor ? `?epoch=${encodeURIComponent(cursor.epoch)}&revision=${encodeURIComponent(cursor.revision)}` : "";
          const response = await fetch(`/api/runtime${query}`,{signal:controller.signal});
          if (!response.ok) throw new Error(await response.text());
          const update:Runtime.Update = await response.json();
          if (stopped) return;
          const previous = new Map(snapshot?.records.map(record=>[record.id,record]) ?? []);
          snapshot = applyRuntimeUpdate(snapshot,update);
          for (const record of snapshot.records) {
            if (record.context.activation_id !== id || JSON.stringify(previous.get(record.id)) === JSON.stringify(record)) continue;
            const state = record.state;
            const phase = state.kind === "activation" && ["succeeded","failed","cancelled","interrupted"].includes(state.status) ? state.status
              : state.kind === "port" ? `flow.token.${state.action}` : `runtime.${state.kind}`;
            listener({activation_id:id,node_id:record.context.node_id,sequence:++sequence,phase,detail:state});
          }
          const terminal = snapshot.records.some(record=>record.context.activation_id===id && record.state.kind==="activation" && ["succeeded","failed","cancelled","interrupted"].includes(record.state.status));
          if (!terminal) timer=setTimeout(()=>void poll(),100);
        } catch(error) {
          if (!stopped && error instanceof RuntimeResyncRequired) { snapshot = null; timer = setTimeout(() => void poll(),100); return; }
          if (!stopped) listener({activation_id:id,sequence:++sequence,phase:"transport_error",detail:String(error)});
        }
      };
      void poll();
      return () => {
        stopped = true;
        controller.abort();
        clearTimeout(timer);
      };
    },
  },
};
