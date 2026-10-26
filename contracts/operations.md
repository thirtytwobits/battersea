# Operations

## Contribution

`battersea_ops::BatterseaOps` contributes Battersea's operations to
[Clapham](https://github.com/thirtytwobits/clapham) under the `battersea` namespace. Each host
capability the application supplies contributes its operations; a capability left out
contributes none. Every operation has `daemon` locality and runs under the caller's verified
OAuth grant.

| Operation | Effect | Scope | Capability |
|---|---|---|---|
| `battersea.flow.list`, `.get`, `.validate` | query | `battersea:flows:read` | `FlowHost` |
| `battersea.flow.upgrade` | administrative, preview | `battersea:flows:admin` | `FlowHost` |
| `battersea.run.activate` | mutation, streams | `battersea:runs:write` | `RunHost` |
| `battersea.run.status` | query | `battersea:runs:read` | `RunHost` |
| `battersea.run.watch` | query, streams | `battersea:runs:read` | `RunHost` |
| `battersea.run.cancel` | mutation | `battersea:runs:write` | `RunHost` |
| `battersea.run.resume` | administrative | `battersea:runs:admin` | `RunHost` |
| `battersea.approval.list` | query | `battersea:approvals:read` | `ApprovalHost` |
| `battersea.approval.decide` | mutation | `battersea:approvals:decide` | `ApprovalHost` |
| `battersea.approval.reconcile` | administrative | `battersea:approvals:admin` | `ApprovalHost` |
| `battersea.mcp.servers` | query | `battersea:mcp:read` | `McpHost` |
| `battersea.mcp.refresh` | administrative | `battersea:mcp:admin` | `McpHost` |

The host interfaces of the runtime, the journal store and the tool approval store are unchanged;
the application adapts its own hosts to these capabilities.

## Authority

An operator's grant and an agent's grant for the same person carry different scopes; the
authorisation server issues them. Agent grants carry no `:admin` scope and no
`battersea:approvals:decide`.

`run.activate` passes the caller's grant to `RunHost::activate`; the host decides the grant the
run's tool calls are made under, and `ApprovalHost::agent` reports its identity.
`approval.decide` refuses as `own_grant_decision`, with `not_dispatched` evidence and before any
write, when the caller's issuer, subject and client are that identity. The check belongs to the
operation, so composition needs no resource policy for it.

A decision's or reconciliation's actor is the caller's `GrantIdentity` as JSON: its issuer and
subject, with its client where present.

## Refusals

Every operation declares the same refusals, each under its Clapham category: the codes in
`battersea_ops::codes`, the runtime's `error_codes`, flow validation's `issue_codes` and every
`ToolCode`. `codes::declared` lists them. A host builds a refusal with `codes::refusal`, which
takes the code's declared category. A code Battersea does not declare becomes `host_failure`,
carrying the original code in `details`, so no operation returns `undeclared_refusal`.

| Code | Category |
|---|---|
| `unknown_flow`, `unknown_run`, `unknown_mcp_server` | `not_found` |
| `invalid_flow`, `invalid_reconciliation` | `invalid_input` |
| `stale_upgrade_revision` | `conflict` |
| `upgrade_revision_required` | `usage` |
| `own_grant_decision` | `refused` |
| `host_unavailable` | `unavailable` |
| `host_failure` | `internal` |

A durable executor's refusal carries its tool code. A storage refusal reports `uncertain`
evidence, since the write may have committed.

## Flow upgrade

`flow.upgrade` supports a dry run. A preview returns the upgrade `FlowHost::plan_upgrade` would
apply, with `applied: false` and its `source_revision`, and is refused as
`stale_upgrade_revision` when it names a different expected revision. Applying the upgrade
requires `expected_revision`; without it the call is refused as `upgrade_revision_required`
before dispatch. `FlowHost::apply_upgrade` fences on the revision and returns the same shape
with `applied: true`. Reading a flow never upgrades it.

## Streams

`run.activate` and `run.watch` send each execution event through the call's record sink, then
return the run's status. The CLI prints records as NDJSON. Over MCP they ride Clapham's records
extension, `io.github.thirtytwobits.clapham/records`; a client without the extension receives
the final result alone.

## Connections

`mcp.refresh` asks the host to rediscover a connection's catalogue and rebind it. A composed
set is immutable, so the host recomposes and swaps the set it serves. New runs take the new
selection; a running conversation keeps its pinned binding, which the stale check fences.

## Conformance

`examples/ops` is a deterministic host and the reference executable Clapham's conformance
harness drives. Every operation, and its refusals, gives the in-process result through the CLI
against its daemon over Streamable HTTP and stdio, raw MCP over both transports, and the MCP
client bound to each, with records compared in order.
