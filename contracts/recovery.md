# Durable execution recovery

Status: implemented. Execution and scheduler checkpoints use version 2.

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

## Handled node failures

The accepted graph pins each authored failure policy and the catalogue's failure ports.
The scheduler checkpoint retains each handled node's failure identity, phase, operation,
selected policy and payload, failed stream closures, settled ports and pending emissions.
The checkpoint retains the reason for each dependency skip: failed input or absent snapshot
value. Every stopped node requires either a handled failure record or a dependency skip
record. Restore validates these records against the pinned graph, settled inputs and payload
limits.

Handling is part of the enclosing driver segment. Publish its effect results and the
checkpoint together. Before that publication, interruption leaves the segment's outstanding
intent requiring reconciliation. A handled node error cannot resolve an uncertain tool,
provider or host effect. A failed checkpoint publication stops execution, including when
the storage write may already have committed.

After publication, resume uses retained deliveries and closures. It neither invokes the
handled node again nor constructs another error token. Already consumed error deliveries
and settled signal/action latches remain consumed and settled. Failure identities survive
observation attempts; observation event identities retain their attempt component.

Execution and scheduler checkpoints use version 2 for failure state. Unsupported versions remain
inspectable and refuse resume. Inspection cannot upgrade a journal or replace its pinned
graph. Explicit authored-flow upgrades apply to new activations.

Required proof includes interruption before failure classification, after settlement and
before/after checkpoint publication, including commit followed by lost acknowledgement.
Reopen through a new driver and verify retained ports, ordered fan-out, failure identities
and effects, or an explicit unresolved intent.

## Retention

A journal is read while a command can still act on it. Once none can, the host may remove it.

- A recovery journal is read until it is terminal, committed or abandoned
  (`JournalRecord::is_terminal`). Every command refuses a terminal journal.
- A tool-approval journal and a conversation checkpoint serve a run, which reads their retained
  results when it resumes. Each is read until it is settled and that run has ended; under a
  recovery journal, until the journal is terminal.
- A tool-approval journal is settled when every call is denied, cancelled, completed, failed or
  abandoned (`ToolApprovalJournal::is_settled`).
- A conversation is settled when it is completed, or abandoned with no remote response left to
  cancel (`ConversationCheckpoint::is_settled`). `cancel` reads an abandoned conversation that
  retains a remote identity until it publishes terminal remote evidence.

## Required proof

Inject failure before/after acceptance, intent, dispatch, result/checkpoint publication and host
commit. Reopen storage using a new driver. Observe either continuation from retained state with
no repeated effect or an explicit unresolved intent. Also cover competing revisions, changed
configuration, unknown versions, corrupt or oversized state, cancellation and terminal retries.

Each explicit resume starts a new observation attempt. Event identities combine run, attempt
and sequence so observations from an interrupted attempt cannot be overwritten.

## Tool approvals

The [tool approval journal](tools.md#durable-approval) persists pending requests, host decisions,
dispatch intents and results through application-owned storage. Each conversation has its own
revision-fenced scope and expiry. A recovered tool result can satisfy the host's retained
continuation; the enclosing driver intent still requires its own checkpoint and publication.


## Provider conversations

The [provider conversation checkpoint](providers.md#durable-tool-conversations) retains the
accepted request, completed provider turns, tool batches and copied results. Provider dispatch
has its own persisted intent. An unresolved provider intent requires reconciliation even when
all preceding tool effects have published results. Completing a provider conversation supplies
the enclosing driver with retained output; the driver still publishes its own checkpoint.

Background provider acceptance retains the remote identity before retrieval. A failed retrieval
can repeat against that identity; an uncertain create requires reconciliation. Cancellation records
its own intent and can repeat idempotently until terminal remote evidence is published. The host
owns remote cleanup and may cancel after conversation expiry. Reopening preserves the response
identity, original expiry and consumed generation budget.
