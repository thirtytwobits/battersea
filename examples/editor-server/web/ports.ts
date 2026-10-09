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
      let sequence = -1;
      const controller = new AbortController();
      const poll = async () => {
        try {
          const response = await fetch(
            `/api/activations/${encodeURIComponent(id)}`,
            { signal: controller.signal },
          );
          if (!response.ok) throw new Error(await response.text());
          const snapshot = (await response.json()) as {
            status: string;
            events: FlowDiagnostic[];
          };
          if (stopped) return;
          for (const event of snapshot.events)
            if (event.sequence > sequence) {
              listener(event);
              sequence = event.sequence;
            }
          if (snapshot.status === "running")
            timer = setTimeout(() => void poll(), 100);
        } catch (error) {
          if (!stopped)
            listener({
              activation_id: id,
              sequence: sequence + 1,
              phase: "transport_error",
              detail: String(error),
            });
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
