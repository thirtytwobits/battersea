# Battersea runtime

`NodeHandler` implements source, inline, sink, action, controller and receive hooks.
`ExecutionHost` supplies the application state and effects. `ActivationHost` adds durable
acceptance and completion hooks to the shared preflight, execution, source and drain cycle.

The driver records `CompletionPending` after execution. The host reports `Succeeded` only
when its commit is durable. Recovery reconciles accepted effects and retained completions;
it does not redispatch accepted provider work.

## Delivery and ownership

Version 2 documents declare execution limits, source priority and streaming mailbox policy.
Catalogue ports declare consumption mode and phase. The driver admits every lossless
fan-out branch before publishing an emission, delivers branches in edge order, and closes
outputs after their phase completes. A failed branch prevents later branch delivery.

Final inputs return `Retained<Token>` from `take_flow_input_token`. Sinks receive the same
owned handle. Cloning a handle shares its immutable payload and charge until the last handle
drops. Streaming receive hooks consume each delta once; final execution waits for successful
closure of every connected stream. A hybrid materialises snapshot outputs separately from
its execution outputs.

`start_provider` registers an owned stream of `ProviderEvent`. Its pump reserves capacity
before polling and retains reservations through handling. The driver applies token, signal,
custom data and failure events. Custom data reaches `receive_provider_event`;
`provider_complete` can register a tool continuation. The pump cannot borrow graph state.
Ready providers are selected fairly, and the event log records their selected order.

A provider reserves its configured mailbox item and byte capacities from the activation
budget for its lifetime. Edge deliveries, control events and retained application values
share the remaining activation budget. Set limits to accommodate the provider mailboxes
and their downstream fan-out together. Capacity pressure runs queued consumers; a blocked
reservation with no runnable consumer reports the affected port or edge. Cancellation
interrupts blocked handlers and disposes owned pumps.

Application parser buffers, collected text, tool results and controller snapshots use
`runtime.retention.retain(node, value)`. Replacements are admitted before releasing the old
value. This accounts for encoded payload bytes, including delivery causes; it does not
measure process RSS. Event storage has its own host-owned retention policy. Custom handlers
must account for payloads they copy out of received values.

`examples/custom-node` implements a complete host without a product session.
`examples/editor-server` demonstrates an owned provider pump, explicit document upgrades,
and durable completion through its HTTP and CLI clients.
