# Implement a node

Run the independent custom-node application:

```sh
cargo run --locked -p custom-node-example
```

Its `Source` and `Sink` implement `NodeHandler<Application>`. Each declares its ports,
consumption modes, and execution phase through the `battersea_derive::NodeDefinition` derive.
The derive supplies metadata; the handler implements execution.

The application registers both definitions and handlers in its own registry. It supplies
the state, typed error implementation, cancellation token, acceptance, and completion hooks.
Inspect `examples/custom-node/src/main.rs` in the matching release for the full implementation.

Use [`NodeHandler`](../rust/battersea_runtime/trait.NodeHandler.html) for execution hooks and
the [runtime reference](../rust/battersea_runtime/index.html) for host and registry interfaces.
The [text-source contract](contracts/text-source.md) describes definition registration checks.

Keep token retention and external effects explicit. A custom in-process handler must honour
the runtime's accounting rules; it is not a memory sandbox. The [execution contract](contracts/execution.md)
defines delivery, backpressure, and cancellation requirements.
