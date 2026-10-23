# Execution contract

Status: implemented. Saved flows use document version 3, reached through an explicitly
invoked upgrade.

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
  Successful closure without a value settles that input as absent. A node requiring
  that value does not execute: its execution outputs close without values and its signals
  settle false. Absence propagates through dependent branches; independent branches
  still complete. A sink's final-value dependency settles on delivery or successful closure.
  An input whose producer has not closed remains unresolved and cannot be treated as absent.
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
capacity before polling another transport frame. A receiver releases capacity after
handling or discarding that frame. Provider payloads are fragmented and reassembled
losslessly; frame capacity does not restrict the size of a tool result. Encoded
payloads, partial assembly and borrowed complete events use the node retention budget.
Only complete events reach handlers. The next logical event is polled after the
previous delivery is released. Cancelling a receive future preserves partial assembly;
cancelling the activation or dropping the pump releases it.

`backpressure` suspends the producer, while the driver continues runnable consumers and
cancellation. `drop_oldest` is allowed only on explicitly lossy streaming edges; every
discard records its edge, sequence and byte count. Final-value, automation and signal
delivery are lossless. A payload that exceeds its retention budget fails explicitly. M6.1 supplies finite limits;
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
before making any branch observable. Delivery follows edge order. An unhandled destination failure
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

## Node failure policies

Each node authors `failure_policy`: `fail`, `emit` or `skip`. Omission means `fail`.
Only explicitly classified, recoverable node failures are eligible. Classification carries
a stable host code and a sanitised message and confirms that external effects have settled.
Unclassified errors and runtime-owned refusal codes retain their original outcome.
Cancellation, capacity admission, contract violations, panics, journal failures and unresolved
effect intents bypass node policies. Classification does not authorise retry or effect replay.

| Policy | Eligible failure |
|---|---|
| `fail` | Fail the activation and stop subsequent branch delivery |
| `emit` | Stop the node for this activation, publish one typed failure and settle its ports |
| `skip` | Stop the node for this activation and settle its ports |

The driver handles a failure at its originating callback: source materialisation, direct
execution, streaming receive, sink receive, action, controller action, provider event or
provider completion. Nested delivery retains the destination's attribution and policy.
Standalone controller calls return their error to the caller; graph policies require an
activation. Automation rejection retains its separately defined outcome.

A handled node performs no further callbacks in the activation, including later hybrid
phases. Its provider pump stops, scheduler-owned inputs and pending deliveries to the node are
released, and subsequent deliveries to that node are discarded with an observation.
Previously admitted outputs and emitted signals retain their order. Published final values
remain valid; unpublished final outputs close absent. Streaming outputs close with handled
failure after their admitted prefix. A dependent node requiring successful stream closure
skips its remaining execution and propagates absence. Sinks retain effects already applied
to the delivered prefix. Independent branches continue.
An absent snapshot final value skips dependent materialisation and execution; its stream
outputs close successfully without values.

The driver settles unissued signals false and suppresses `post_activate` and success-only
controller effects. A handled middle fan-out destination permits later destinations to
receive the admitted token in edge order. Earlier destinations are never redispatched.

Catalogue `failure_ports` declare a unique name and `snapshot` or `execution` phase.
Each expands to a runtime-owned, final-value `node_failure` output. Names share the ordinary
output namespace; each phase has at most one failure port. `emit` requires a failure port
for every phase the node can execute. Ordinary handlers cannot publish on failure ports.
Failure ports close absent on successful execution and under `skip`. Under `emit`, the
failing phase publishes exactly one value; other failure ports close absent. Source and
hybrid snapshot failure ports retain snapshot dependency ordering.

The `node_failure` payload contains `id`, `node_id`, `phase`, `operation`, `code` and
`message`. Its identity combines the run, node and first failure's delivery sequence and
is retained by recovery. Code and message are bounded to 128 and 4096 UTF-8 bytes.
Hosts sanitise messages before classification. Failure output admission uses the activation's
existing limits, FIFO ordering and fan-out reservations. Publication and closure follow
previously admitted emissions. Admission and closure failures retain their control outcome.
A node cannot reapply its policy to its own failure publication.

The driver records the original failure, selected policy and handling result under the
originating node. Handled node failure and activation outcome are separate observations.

## Authoring

The editor exposes source priority, activation limits, provider capacity and connection
order. Stream connections also expose item and byte capacities, maximum event size and
an explicit overflow policy. Catalogue mode and phase are read-only. Invalid capacities
remain in the form until corrected; applying valid settings participates in undo/redo.
Draft restoration, validation, save, clone and reload preserve authored settings.
The node inspector exposes the failure policy and catalogue-declared failure ports.
Policy changes participate in undo/redo and persist through draft restoration, clone,
save and reload. The complete graph is validated before activation.

## Upgrade and verification

The explicit upgrade records the source order produced by the v1 materialisation rule,
assigns port modes from the pinned catalogue, and adds finite execution limits. It
reports ambiguous custom-node modes for author resolution. It preserves edge identity,
edge order, parameters, extension metadata and layout. Reading, importing, listing or
validating v1 retains its bytes and reports incompatibility. The host fences the upgrade
by revision and preserves a recoverable original before replacing a stored document.
Session-bound graph copies require the host's explicit session upgrade as well.

The v2-to-v3 upgrade preserves authored fields and assigns the default `fail` policy.
The v1 upgrade includes that step. Inspection, import, listing and validation preserve
older bytes; replacing a stored flow requires explicit, revision-fenced host publication.

| Clause | Distinguishing proof |
|---|---|
| Source order | Permute node arrays and titles; observe unchanged declared order; reject dependency cycles |
| Streaming/final | Empty stream closes; all deltas delivered once; final fires after closure; second final rejected |
| Pump independence | Consumer blocked while producer reaches capacity; consumer can resume; cancel wakes both |
| Retention | Hold a received delivery; producer cannot refill its permits; oversize payload fails before delivery |
| Fan-out | Permute edge arrays; observe declared order; fail an unhandled middle branch; no later delivery or redispatch |
| Signals | Full data queues cannot prevent cancellation; preflight disable precedes dispatch; cycle diagnostics |
| Failure policies | Fail/emit/skip across every callback; handled middle fan-out continues; partial streams skip final execution |
| Failure boundaries | Cancellation, capacity, journal and unresolved effects retain their control outcomes |
| Upgrade | Read/import leave bytes, timestamps and ownership unchanged; explicit fenced upgrade retains original |
| Application | Chat, compaction, refinement, story memory and media preserve attributed outputs and durable completion |

Playback of recorded events, deterministic re-execution and effectful resume are separate
operations. Quiescent queues do not establish durable completion of external effects.
