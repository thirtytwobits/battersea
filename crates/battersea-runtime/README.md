# Battersea runtime

`NodeHandler` implements source, inline, sink, action, controller and receive hooks.
`ExecutionHost` supplies the application state and effects. `ActivationHost` adds durable
acceptance and completion hooks to the shared preflight, execution, source and drain cycle.

The driver records `CompletionPending` after execution. The host reports `Succeeded` only
when its commit is durable. Recovery reconciles accepted effects and retained completions;
it does not redispatch accepted provider work.

`examples/custom-node` implements a complete host and custom nodes without a product session.
