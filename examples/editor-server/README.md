# Editor application

The Rust application supplies a custom text node, filesystem documents and activation records.
The browser uses the installed `@battersea/editor` and `@battersea/flow` packages. Its CSS and
HTTP adapter belong to the application. The daemon also serves Battersea's flow and run
operations through Clapham's MCP server. The CLI is Clapham's, assembled from those operations,
and calls the daemon over MCP.

Build the server with Cargo, initialise an empty application directory, then start the server.
The server's `--help` describes its commands. In `web/`, install the release packages with
`npm ci` and start Vite with `npm run dev`. Vite proxies `/api` to the loopback server.

`init` writes local test tokens to `tokens.json`: an operator's, holding every scope the example's
operations require, and an agent's, holding none of the administrative scopes. The daemon accepts
each token as its grant. On start it writes its endpoint record, `daemon.json`, holding the MCP
endpoint and the operator's token; both files are readable only by their owner. The CLI finds the
daemon through `--root` and that record, or through `--endpoint` and `--token`
(`BATTERSEA_ENDPOINT`, `BATTERSEA_TOKEN`):

```sh
editor-server-example --root /tmp/battersea-example battersea run activate example text \
  --parameters-json '{"text":"A CLI-authored input"}'
```

The activation prints each execution event as an NDJSON line, then its outcome. With a daemon
running, `battersea --help` lists the operations the token's grant can discover. Administrative
operations, such as `battersea flow upgrade`, require `--yes`, including for `--dry-run`.

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
| `/mcp` | Battersea's flow and run operations over Streamable HTTP, under a local token's grant |

The browser polls an activation until terminal state and detaches on unmount. Primrose supplies
its own worker/projection adapter to the same editor contracts.
