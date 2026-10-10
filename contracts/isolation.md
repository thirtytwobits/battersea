# Node isolation evaluation

Status: evaluated 2026-10-10. Nodes are trusted Rust code linked by the application. The consuming
Primrose application requested evaluation, without untrusted-node execution. A sandbox ABI needs
a separate design tied to an actual isolation requirement.

## Trust and candidates

A native handler shares process memory, OS authority and credentials. Panics, cooperative
cancellation and scheduler retention budgets do not contain arbitrary native code or infinite
loops. Loading a third-party crate also executes build scripts and procedural macros during the
build; runtime isolation cannot protect that build pipeline.

WebAssembly is a candidate for untrusted computation: memory accesses are checked and external
operations require linked imports. The embedder still decides the authority of those imports.
WASI filesystem access is capability based. These properties favour an explicit host interface
rather than exposing the Rust host trait directly. See [Wasmtime security](https://docs.wasmtime.dev/security.html).

An OS worker process is a candidate when a node requires native libraries. A process alone does
not constrain filesystem or network access; a platform-specific permission policy, IPC protocol
and termination/reaping design are required. WASM and OS workers both need host-side effect and
resource accounting. Neither candidate is selected by this evaluation.

## Required design and proof

| Boundary | Requirement before enabling untrusted nodes |
|---|---|
| Admission | Identify module bytes, ABI, catalogue and state versions; verify package provenance; isolate untrusted builds |
| Capabilities | Grant named operations per activation; keep credentials, storage paths, session objects and raw sockets in the host; validate every payload and handle |
| Resources | Bound guest memory, stack, tables, compilation, CPU, wall time, host-call concurrency, outputs and retained handles; charge copied host data |
| Cancellation | Interrupt guest execution and cancel/settle host effects; reclaim handles; test infinite loops and blocked imports |
| Effects | Route providers, tools and writes through journalled host operations; preserve uncertain outcomes and explicit recovery |
| Checkpoints | Serialize bounded guest state; pin module and ABI identity; reject unsupported versions; retain explicit upgrades |
| Errors | Map traps/crashes into attributed failures; contain malformed messages and terminal control sequences; keep host state coherent |
| Conformance | Test forbidden filesystem/network access, exhaustion, denial of service, stale handles, crash timing and cross-activation leakage on supported platforms |

Wasmtime exposes fuel and epoch interruption for guest execution; host calls need their own
cancellation policy. Its resource limiter covers selected WebAssembly allocations, not every host
allocation. These are components of a design, not a complete resource policy.
[Execution configuration](https://docs.wasmtime.dev/api/wasmtime/struct.Config.html#method.consume_fuel),
[resource limiter](https://docs.wasmtime.dev/api/wasmtime/trait.ResourceLimiter.html).
