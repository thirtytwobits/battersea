# Integrate a host

Your application owns sessions, durable storage, product tools, media, and presentation.
Battersea owns graph validation and generic execution. Public packages do not depend on
a consuming product.

1. Build a catalogue with your handler IDs, token schemas, parameter schemas, and validators.
2. Implement `ExecutionHost` with application state and effects.
3. Implement `ActivationHost` when you need durable acceptance and completion.
4. Register handlers and submit a validated flow with finite execution limits.
5. Persist completion before reporting success to the application.

An activation pins accepted inputs and revisions. After execution, completion can remain
pending until the application's commit is durable. Recovery reconciles accepted effects;
it must not redispatch accepted provider work. Implement the ports in the
[recovery contract](contracts/recovery.md) before enabling durable execution.

The [runtime guide](runtime.md) explains delivery and ownership. For accounting, bounded
observation, and application ledger responsibilities, read the [observation contract](contracts/observability.md).
Provider retry and dispatch certainty follow the [provider contract](contracts/providers.md).

The standalone editor server demonstrates filesystem documents, revision fencing, explicit
upgrades, activation endpoints, and durable completion. See [examples](examples.md).
