# Kits

Status: implemented; designed in [#70](https://github.com/thirtytwobits/battersea/issues/70).
`battersea-nodes` is the generic kit.

A kit is a domain-specific set of nodes that applications share. It is one Cargo crate. Its
handlers reach the application only through its port, so an application that implements the
port runs the kit's nodes unchanged, and the kit's conformance suite checks that implementation.

## Parts

| Part | Form | Required |
|---|---|---|
| Descriptor | `pub const KIT: battersea_kit::Kit` | yes |
| Definitions | `MANIFEST` and `register_token_types` | yes |
| Handlers | `register_handlers<H: Port>` | yes |
| Port | `pub trait Port: ExecutionHost` | when handlers need application state |
| Reference host | `impl Port for battersea_kit::ReferenceHost<reference::State>` | with a port |
| Contract | `contracts/kit-NAME.md` in this repository; `CONTRACT.md` elsewhere | yes |
| Conformance | `conformance::run`, behind the `conformance` feature | yes |
| Editor package | `@battersea/kit-NAME`, parameter renderers and controller widgets | when definitions need them |

`battersea-kit` is a public crate supplying `Kit`, the in-memory `ReferenceHost` and the
conformance harness. It depends on `battersea-flow`, `battersea-runtime` and `battersea-model`.

## Names

A kit's name matches `[a-z][a-z0-9_]*` and is its namespace: every handler id is `NAME.*`,
every token type it registers is `NAME.*`, and every class name begins with the name in
PascalCase. `Kit::check` refuses a manifest that breaks these rules. Class names and handler ids
are persisted in flows and are never renamed; a rename is a registered document upgrade.

`battersea` is the generic kit's name. Its class names and the token types `prompt.fragment`,
`prompt.fragmentArray` and `chat.raw` are the shared vocabulary every kit may reference, and the
only names outside a kit's namespace it may register or reference.

A kit in this repository is the crate `battersea-kit-NAME`, with its manifest at `src/nodes.yaml`,
its contract at `contracts/kit-NAME.md` and its example host under `examples/kit-NAME`. Its
editor package, when it has one, is `@battersea/kit-NAME` under `typescript/packages/kit-NAME`.

## Port

The port is a trait with `ExecutionHost` as its only supertrait, written in the kit's terms:
the records it stores, the services it calls, the state it keeps in a run. It names no other
kit's port, no provider, no transport and no product type. Handlers are generic over
`H: Port`; the kit never names a concrete host.

The port projects the kit's run state out of the host's `State`:

```rust
pub trait Port: ExecutionHost {
    /// What the application adds to the kit's records.
    type Extension: Serialize + DeserializeOwned + Clone + Send + Sync;
    fn run_state(state: &mut Self::State) -> &mut RunState<Self::Extension>;
    // the kit's own operations follow
}
```

A kit's record and token types carry the extension at a reserved `extension` key.
`register_token_types(registry, &Options { extension_schema })` embeds the application's
schema there; the reference host uses `true`. A kit's `RunState` declares `STATE_VERSION`; the
application includes it in its versioned host state, and resume refuses another version.

A host implements `ExecutionHost` once and each kit's port independently. Ports compose by
conjunction of bounds; there is no port hierarchy. Two kits share data through tokens of the
shared vocabulary, never through each other's ports.

## Reference host

`battersea_kit::ReferenceHost<X>` is an in-memory `ExecutionHost` whose `State` carries the
kit's `X` and whose clock is a `ManualClock`. A kit implements its port for
`ReferenceHost<reference::State>`. The reference host is the kit's own test host and the oracle
its conformance suite compares against. It makes no durable commitment and has no checkpoints.
A kit that registers parameter datatypes builds the reference host `with_registry`, so its
automation writes convert as an application's registry does.

## Conformance

A kit's contract has a distinguishing-proof table, as the execution contract does. The kit's own
tests prove each clause against the reference host. The conformance suite then proves that an
application's port implementation behaves as the reference host does.

Each case is a flow in [text source](text-source.md), activation values, the clock steps taken
during the run, and the port operations performed before it. `conformance::run(&fixture)` plays
every case on the reference host and on the application's host, both on a `ManualClock` the
suite owns, and reports the first divergence in outcome, execution events and the port's
observable state. Emitted tokens are compared through the runtime's receive events, which
carry each value. The suite reaches the application only through `Port` and
`battersea_kit::conformance::Fixture`:

```rust
pub trait Fixture {
    type Host: ActivationHost;
    /// A host with no state, running on `clock`.
    fn host(&self, clock: Arc<ManualClock>) -> Self::Host;
    /// The host's run state around `scheduler`, with no kit state.
    fn state(&self, host: &Self::Host, scheduler: SchedulerState) -> <Self::Host as ExecutionHost>::State;
    /// The execution events the host recorded for `run_id`.
    fn events(&self, host: &Self::Host, run_id: &str) -> Vec<ExecutionEvent>;
}
```

A clock step is taken once the activation waits on a timer, and the next once the activation
has completed or dropped every timer the step reached, so a wait racing a longer timeout
completes. A case whose steps outnumber a host's waits fails, and so does a host that waits
after the last step; neither waits on the wall clock. The kit reaches its port through
`PortAccess<H: Port>`, which applies a case's port operations before the activation and
observes the port's state after it; the kit's `conformance::run` passes it for both hosts, and
a kit without a port passes `NoPort`. Event comparison ignores the host's own error wording: the
summary and error message of a failure record and the message of a terminal outcome. Codes are
compared.

The application adds the kit with the `conformance` feature as a dev-dependency and runs the
suite in a test. A kit in this repository has an example host under `examples/` that implements
its port over something other than the reference host's maps and runs the suite; `cargo xtask
package` compiles it against the packaged crate.

## Model-facing text

A kit's manifest is the only text it ships that reaches a model, and is the kit's prompt root
under the prompt scanner. `Kit::model_facing_text` returns that text keyed by formatter block
and definition field, so an application pins it with a snapshot: a kit upgrade that changes
model-facing text fails the application's test until the text is reviewed.

`Catalog::from_manifests` resolves formatter blocks across all its sources: a block named in a
later source replaces the earlier block of that name, and definitions are never replaced. An
application overrides a kit's text by listing a manifest holding only `formatter_blocks`, with
no `node_definitions`, after the kit's. `model_facing_text` resolves each port formatter to the
template it names, so a changed reference changes the snapshot as a changed block does.

## Releases and dependencies

A kit in this repository is a workspace member at the workspace version, listed in
`release.json`, with an API snapshot and an entry in `contracts/dependencies.json`. A kit may
name `battersea-flow`, `battersea-runtime`, `battersea-kit` and `battersea-model`. It may not
name another kit, `battersea-nodes`, `battersea-providers`, `battersea-config`, `battersea-ops`
or `battersea-tools`: a provider, a tool runner or a store reaches a handler through the port.

A kit elsewhere takes one Battersea release tag and versions itself. Its editor package carries
the crate's version.

## The generic kit

`battersea-nodes` is the kit `battersea`, without renaming. It has `KIT`, the `conformance`
feature and tests on the reference host. Its handlers require only `ExecutionHost`, so it has
no port and its suite proves catalogue composition and handler behaviour on the application's
host: `conformance::PROBE_MANIFEST` adds a source that emits activation values and a sink,
whose handlers the application registers beside the generic handlers in its test host. Other
kits do not depend on it; the application composes it beside them. `battersea_kit` holds the
`NodeError` and `text` helpers kits share, and `battersea-nodes` re-exports them.

## Verification

| Clause | Distinguishing proof |
|---|---|
| Names | `Kit::check` refuses a handler id, token type or class name outside the namespace; accepts the generic kit's vocabulary |
| Port independence | A host implementing two kits' ports runs both; neither kit compiles against the other |
| Extension | Records round-trip the application's extension; the reference host accepts any extension |
| Reference host | Every contract clause passes on `ReferenceHost`; no test reads the wall clock |
| Conformance | The suite passes on the reference host and the example host; a deliberately wrong port implementation fails with the diverging case named |
| Model-facing text | Changing a formatter block changes `model_facing_text`; a later manifest's block replaces it; a later definition of the same class is refused |
| Releases | `cargo xtask check` refuses a kit outside `release.json`, `dependencies.json` or the API snapshots; `cargo xtask package` compiles the example host |
