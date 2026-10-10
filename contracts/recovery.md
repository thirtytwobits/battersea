# Durable execution recovery

Status: implemented in v0.10.0.

An accepted run has one identity, pinned graph/catalogue/configuration revisions, versioned
node and host state, and a revision-fenced journal. Its checkpoint contains a resumable driver
continuation and the complete scheduler state, including admitted deliveries and closure state.
The host owns durable storage and the lifetime of application resources.

## Publication and effects

Persist acceptance before executing. Before an effectful step, atomically append its intent
against the journal revision. A driver segment includes nested dispatch and any providers it starts;
its intent covers that entire segment. Publish its result and the corresponding checkpoint together.
A failed publication stops further work. A storage error may have happened after durable commit;
the next command inspects the record instead of replaying the write's preceding effect.

Checkpoint boundaries have no borrowed driver stack or live provider tasks. While providers are
running, their intents remain outstanding. Their eventual checkpoint includes every selected
event and application-state mutation. An interrupted provider task needs a provider-supported
continuation or explicit effect resolution; it cannot be reconstructed by resubmitting generation.

Recorded completion authorises host reconciliation, not graph execution. The host commit has its
own intent and result. A commit whose result was not recorded must be inspected at its durable
destination before retrying. Reconciliation preserves the run identity and produces one outcome.

## Inspection and commands

Loading a journal does not create, repair or upgrade it. Unsupported journal/checkpoint/node
versions remain readable as incompatible entries. Restore validates identity, revisions,
node-state versions, graph structure and payload bounds before executing anything.

Resume and resolution use an expected revision. At most one contender can advance it. The host
also fences active execution ownership; a journal revision is not a distributed lease. A resumed
driver stops on any rejected write before performing the next effect.

An outstanding intent is uncertain, including one interrupted between intent persistence and
dispatch. Resolution records an actor and evidence. A confirmed not-applied result permits the
step to run from its preceding checkpoint; abandonment terminates the run. A completed effect
is reconciled through the owning host's retained result, never by inventing output from a missing
record. Terminal runs cannot resume or change outcome.

## State and resource ownership

Every restored payload is admitted through the same item/byte limits as initial execution.
Queued deliveries preserve their order, fan-out grouping and causal signal path. Completed nodes,
settled signals and closed ports remain completed, settled and closed. Host snapshots carry an
explicit schema version and per-node handler-state versions.

Configuration is pinned by identity; restoring against another configuration fails. Graph and
catalogue snapshots belong to the accepted run. Restoring does not select a newer authored graph.
Resource expiry and ownership are host invariants: recovery cannot renew or transfer them as a
side effect of inspection, nor manufacture a successful host commit from scheduler quiescence.

## Required proof

Inject failure before/after acceptance, intent, dispatch, result/checkpoint publication and host
commit. Reopen storage using a new driver. Observe either continuation from retained state with
no repeated effect or an explicit unresolved intent. Also cover competing revisions, changed
configuration, unknown versions, corrupt or oversized state, cancellation and terminal retries.

Each explicit resume starts a new observation attempt. Event identities combine run, attempt
and sequence so observations from an interrupted attempt cannot be overwritten.
