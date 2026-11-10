# Runtime observation and accounting

Node failure observations retain the selected policy and whether handling completed.
A handled node remains failed while its activation can succeed. These optional fields
extend node records; existing records retain their representation when read.

Status: implemented; the independent editor server exposes the runtime view.

Emit records list the edges they leave along, in delivery order, as
`targets: [{edgeId, targetNodeId, targetPort}]`. Receive, skip, drop and input close records name
their edge as `edgeId`.

## Edge marking

An edge's marking is what the edge holds: the deliveries queued on it and, for a token edge, the
close that ends its stream. A close is pending while it waits behind the edge's queued deliveries
and delivered once the target's input has closed; a failed close ends the stream of a failed
source. Signal edges hold no close.

The scheduler owns the marking and publishes it as `flow.edge.marking` records,
`{edges: {EDGE_ID: {queued, close?: {delivered, failed}}}}`, after every change to an edge's
contents: a queued delivery, signal or close, a delivery taken for its target, a delivered close,
and deliveries discarded for a stopped node. Each entry replaces that edge's previous entry. A
restored activation and an ended activation publish every edge; an activation ends with no edge
holding a delivery, and a pending close it discards leaves its edge.

The marking obeys these invariants:

- An edge holds only deliveries emitted along it, and no more than its queue admits.
- When a delivery is recorded, its edge holds what was emitted along it less what was delivered or
  dropped.
- A delivered close follows every delivery on its edge.
- A close moves from pending to delivered and keeps whether it failed. A delivered close is final.

The runtime view keeps one `marking` record per activation, `marking:ACTIVATION`, naming each edge
that holds a delivery or a close. The record is pinned while its activation is active. A view
client draws an edge's marking from the latest `marking` record; a snapshot holds the marking at
its cursor.

`crates/battersea-runtime/fixtures/edge-marking.json` holds the records of one run and
`crates/battersea-telemetry/fixtures/edge-marking-view.json` the view records they produce. The
editor's edge marking is tested against both.

`battersea-telemetry` owns typed records and revisioned views independently of tracing.
Hosts serialise writes; reads never evict or mutate. Retention bounds records, encoded bytes,
delta count and delta bytes. Expiry is a lifecycle write. A foreign, expired or future cursor
requires a snapshot; clients reject non-contiguous deltas atomically.

Lossless capture is a fallible callback invoked before a view mutation becomes visible. Hosts
persist the complete delta or fail capture. A lossy exporter is never a substitute for the
operation journal. OTLP calls have finite batches, response sizes and timeouts, report partial
success as failure, and leave retry ownership with the host.

Content capture requires a mask callback. Hash and byte length describe the original content;
only the callback's output enters the runtime record. Exporters consume those records.

GenAI attribute/metric vocabulary is pinned to
[`6fd0d763a092db245a54104fc1124031eb3c51d2`](https://github.com/open-telemetry/semantic-conventions-genai/tree/6fd0d763a092db245a54104fc1124031eb3c51d2/docs/gen-ai).
The exporter labels that revision. Trace and span IDs are deterministic digests of the host's
unique activation/resource identities. The host exports each completed resource once.

Prices are integer currency micros per million tokens plus a request charge. Input usage
includes cache reads/writes; output includes billed reasoning. Distinct cache rates require
known counts. Reported cost takes precedence. Incomplete usage or absent prices produces an
unknown cost. Arithmetic is checked and rounds charges upwards.

A host supplies a finite bound covering every provider turn, serialises admission, and persists
the next ledger before dispatch. Every applicable budget reserves atomically. Reusing a request
identity is a conflict, including after settlement. Settlement is idempotent. Unknown cost retains
the reservation; only explicit reconciliation releases it. Actual provider overruns are charged
in full and prevent further admission. Restoring a ledger checks its version and reconstructs
its totals. Capacity exhaustion fails admission instead of discarding idempotency identities.

The editor server uses a bounded view and serves `Update` values from `/api/runtime`.
`BATTERSEA_OTLP_ENDPOINT` enables its terminal observation export. The browser follows a cursor
and requests a fresh snapshot after a gap. Execution and journal outcomes remain independent
of collector delivery.
