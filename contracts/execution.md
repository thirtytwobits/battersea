# Execution contract

Status: implemented for v0.6.0. Saved flows require an explicitly invoked upgrade to
flow document version 2.

## Ownership and admission

Battersea owns scheduling, delivery, consumption modes, ordering and resource admission.
The application owns sessions, storage, controller state and durable effects. A provider
pump owns its stream and sends events to the activation driver; it cannot borrow mutable
graph state or call a node handler. Only the driver applies events to graph state.

The driver validates the complete graph, catalogue, port modes and resource limits before
accepting an activation. The accepted identity pins their revisions. A completion remains
pending until the host's durable commit succeeds.

## Ordering

Each flow declares a total source order by node identity. Every snapshot source occurs
exactly once; duplicates, omissions and unknown nodes are invalid. Source dependencies
take precedence over this order. The driver selects the earliest declared source among
ready sources. Only inputs declared for the snapshot phase create source dependencies;
final-execution inputs do not order materialisation. The upgrade maps v1 source-display
inputs to snapshot-phase inputs. A source-dependency cycle is an error. Renaming a node's title, changing layout,
or permuting the stored node array does not change execution order.

Edges from one token output are delivered in `(order, edge id)` order. Each input has at
most one incoming edge. Tokens from a producer retain FIFO order, including across
fan-out. All destinations for token N are admitted before token N+1. Independent running
providers have no deterministic relative arrival order: the driver records the selected
event sequence. Deterministic re-execution requires recorded inputs and that sequence;
graph topology alone does not provide it.

Inline nodes fire once per activation after every connected final-value input is
available and every connected streaming input has closed successfully. Disconnected
inputs do not prevent firing. Input presentation follows `(edge order, port name,
edge id)`; handlers may declare a more specific ordering such as Concatenate's authored
port order. Delivery never starts another node inside a producer's poll operation.

## Consumption

Port mode is separate from nominal token type:

- A final-value output publishes at most one value per activation. A final-value input
  retains that value until its node fires. Multiple values fail the activation.
- A streaming output publishes zero or more deltas followed by successful closure or
  failure. A streaming input processes each delta once through its receive hook and
  releases the delivery afterwards. It does not also retain a copy for final execution.
- Connections require matching modes and compatible token types. A collector is an
  explicit node with a declared retention limit; a streaming edge does not implicitly
  concatenate or turn the last delta into a final value.

A sink handles each admitted delivery once. A hybrid's source phase is independent of
its final execution; an output belongs to exactly one phase, so its successful closure
has an unambiguous owner. Automation receives final values; the write becomes visible when
the driver applies it, before the next scheduled action. A rejected write is recorded.
An empty stream closes normally and does not manufacture a token. Closure cannot
overtake previously admitted deltas. A failed or cancelled stream does not satisfy a
successful-closure dependency.

## Capacity and backpressure

Every streaming mailbox declares positive item and byte capacities, a maximum item
size, and a full-queue policy. Byte accounting covers the declared encoded payload
size, including envelopes; it is not a process-RSS or allocator-overhead guarantee.
Queued and delivered-but-still-borrowed payloads remain charged. Producers reserve
capacity before polling another provider event. A receiver releases capacity only
after handling or discarding that event.

`backpressure` suspends the producer, while the driver continues runnable consumers and
cancellation. `drop_oldest` is allowed only on explicitly lossy streaming edges; every
discard records its edge, sequence and byte count. Final-value, automation and signal
delivery are lossless. An oversize event fails explicitly. M6.1 supplies finite limits;
an unbounded policy cannot satisfy its acceptance gate.

Each activation also bounds pending deliveries and aggregate retained payload bytes.
Fan-out charges each retained copy or shares one charged immutable payload until all
deliveries release it. Application parser buffers, collected text, tool results and
controller snapshots have declared limits and release accounting on replacement or
completion. Event retention is separately bounded by its owner. A custom in-process
handler must honour the SDK's accounting contract; Rust code is not a memory sandbox.

The built-in providers cap each encoded request and cumulative HTTP response body at
64 MiB before decoding. The shared tool loop applies the same bound to accumulated
call/result payloads. Media activity and batch reporters are asynchronous and fallible;
provider work stops when a report fails or its activation is cancelled. Host limits can
be smaller and reject admission explicitly.

A capacity wait never holds the driver's exclusive state. If no runnable consumer or
external producer can release a required reservation, fail with a capacity-deadlock
diagnostic identifying the blocked ports. Cancellation does not require a free data
slot. Closing or dropping a receiver stops its pump and releases reservations. Stopping
a pump does not prove that a remote effect was cancelled.

## Fan-out and signals

Validate all fan-out destinations before admission. Reserve all lossless destinations
before making any branch observable. Delivery follows edge order. A destination failure
fails the activation and prevents subsequent branch delivery; earlier applied effects
remain recorded for host reconciliation. There is no rollback or automatic redispatch.
Lossy branches use only their declared drop policy.

Signals use a separate bounded control queue and are processed by the driver. Emission
settles the source port; delivery latches the target action. Unfired signals settle false
when their source phase completes. Logic evaluates only after every connected upstream
signal has settled. Preserve three-valued preflight logic and disable/cancel semantics.
Within a node turn, outputs and signals follow emission order, so a post-activation
signal cannot overtake that turn's data. Signal cycles fail with their causal path.
Preflight control work drains before provider dispatch. Cancellation has priority over
both queues and wakes producers waiting for capacity.

## Authoring

The editor exposes source priority, activation limits, provider capacity and connection
order. Stream connections also expose item and byte capacities, maximum event size and
an explicit overflow policy. Catalogue mode and phase are read-only. Invalid capacities
remain in the form until corrected; applying valid settings participates in undo/redo.
Draft restoration, validation, save, clone and reload preserve authored settings.

## Upgrade and verification

The explicit upgrade records the source order produced by the v1 materialisation rule,
assigns port modes from the pinned catalogue, and adds finite execution limits. It
reports ambiguous custom-node modes for author resolution. It preserves edge identity,
edge order, parameters, extension metadata and layout. Reading, importing, listing or
validating v1 retains its bytes and reports incompatibility. The host fences the upgrade
by revision and preserves a recoverable original before replacing a stored document.
Session-bound graph copies require the host's explicit session upgrade as well.

| Clause | Distinguishing proof |
|---|---|
| Source order | Permute node arrays and titles; observe unchanged declared order; reject dependency cycles |
| Streaming/final | Empty stream closes; all deltas delivered once; final fires after closure; second final rejected |
| Pump independence | Consumer blocked while producer reaches capacity; consumer can resume; cancel wakes both |
| Retention | Hold a received delivery; producer cannot refill its permits; oversize payload fails before delivery |
| Fan-out | Permute edge arrays; observe declared order; fail middle branch; no later delivery or redispatch |
| Signals | Full data queues cannot prevent cancellation; preflight disable precedes dispatch; cycle diagnostics |
| Upgrade | Read/import leave bytes, timestamps and ownership unchanged; explicit fenced upgrade retains original |
| Application | Chat, compaction, refinement, story memory and media preserve attributed outputs and durable completion |

Playback of recorded events, deterministic re-execution and effectful resume are separate
operations. Quiescent queues do not establish durable completion of external effects.
