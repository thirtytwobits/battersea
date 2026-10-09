# Editor application

The Rust application supplies a custom text node, filesystem documents and activation records.
The browser uses the installed `@battersea/editor` and `@battersea/flow` packages. Its CSS and
HTTP adapter belong to the application. The CLI uses the same activation endpoints as the editor.

Build the server with Cargo, initialise an empty application directory, then start the server.
The server's `--help` describes its commands. In `web/`, install the release packages with
`npm ci` and start Vite with `npm run dev`. Vite proxies `/api` to the loopback server.

The sample stores acceptance and terminal records under `runs/`. Restarting the server retains
those files; its live API addresses activations started by that process. It does not redispatch
accepted work during startup. The separate backend example demonstrates provider and tool registration.

| Endpoint | Contract |
|---|---|
| `GET /api/catalogue` | Released `FlowNodeDefinition[]` |
| `GET /api/flows`, `GET /api/flows/{key}` | Document summaries or a released `FlowDocument` |
| `PUT /api/flows`, `POST /api/flows/clone`, `DELETE /api/flows/{key}` | Explicit document commands; cloning refuses an existing key |
| `POST /api/validate` | Pure catalogue validation |
| `POST /api/activations` | Editor `FlowActivationRequest`; returns its activation ID and state |
| `GET /api/activations/{id}` | State and ordered diagnostics carrying that same ID |
| `DELETE /api/activations/{id}` | Requests server-side cancellation |

The browser polls an activation until terminal state and detaches on unmount. Primrose supplies
its own worker/projection adapter to the same editor contracts.
