# Runtime observation and accounting

Status: implemented; the independent editor server exposes the runtime view.

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
