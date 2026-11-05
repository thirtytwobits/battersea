# Node isolation evaluation

Status: evaluated 2026-10-10. Nodes are trusted Rust code linked by the application. The consuming
Primrose application requested evaluation, without untrusted-node execution. A sandbox ABI needs
a separate design tied to an actual isolation requirement. The threat model below is proposed on
a working assumption, pending a consuming application's requirement.

## Threat model

### Use case

An application installs nodes published by third parties. Neither the application operator nor
the end user trusts the node's author or its code. A guest node computes over tokens: it parses,
transforms, scores and routes values. It reaches providers, tools and storage only through host
operations granted to its activation. Guests implement inline and sink nodes; source, hybrid and
provider-streaming nodes remain trusted Rust. Native libraries, threads, direct filesystem or
network access and GPU access are outside the use case.

### Assets

| Asset | Examples |
|---|---|
| Host authority | Provider and tool credentials, storage paths, session objects, sockets |
| Other activations | Tokens, node state and checkpoints of other runs, users and tenants |
| Host availability | Process memory, CPU, scheduler progress, retention budgets, effect concurrency |
| Durable integrity | Journal intents and results, checkpoints, uncertain-outcome resolution |
| Build pipeline | The host's build machines, CI and release artefacts |
| Downstream readers | Later nodes, the application's UI, logs and terminals |

### Attacker

The attacker is the author of a guest module, or anyone who has compromised its publication. The
attacker:

- supplies arbitrary module bytes and manifest metadata, including ports and descriptions;
- controls all guest computation, and the order, timing, concurrency, size and content of every
  host call and output;
- exhausts or attempts to exhaust memory, stack, tables, compilation, CPU and wall time, loops
  without end or blocks inside a host call;
- sends malformed or oversized messages, forges, replays or retains resource handles, and uses
  them after their activation ends;
- places control sequences and prompt text in outputs and error messages;
- behaves differently per activation, under replay and after recovery;
- traps or crashes at any point, including during a host call and during a checkpoint.

An attacker who controls only a guest's input tokens is weaker: the guest's grants bound what an
exploited guest can do.

### Trust boundaries

| Boundary | Crossing |
|---|---|
| Admission | The host never builds guest source. It admits a WebAssembly binary by digest and verified provenance, then compiles it within bounds. Build scripts and procedural macros run on the author's machines. A precompiled runtime artefact is native code and is never admitted from an author. |
| Host calls | Each import is a named operation granted per activation. Every argument is validated for size, schema, token type and handle ownership. |
| Activation | No guest instance, memory or handle outlives its activation. Guest state persists only as a bounded, versioned checkpoint the host validates on restore. |
| Effects | Provider calls, tool calls and writes are journalled host operations. Credentials never enter the guest. |
| Outputs | Guest tokens pass the port's type and retention admission. Guest-supplied text is data and is escaped for display. |

### Trusted components and exclusions

The application, Battersea, the WebAssembly runtime and the operating system are trusted. An
escape from the runtime is a runtime vulnerability, handled by tracking its security advisories.
The application's choice of grants is its own policy. Microarchitectural side channels between
guests in one process are outside the boundary; an application that needs them closed runs each
tenant's guests in a separate process. Network-level denial of service against the host is
outside the boundary.

### Platforms

Linux on x86_64, where Battersea's CI runs, is the supported platform. Another platform is
supported once the conformance suite passes on it.

### Consequence for selection

The use case excludes native libraries, so a WebAssembly component with no WASI filesystem or
network imports satisfies it. An OS worker process would add a permission policy, an IPC protocol
and process reaping without serving a requirement of this use case.

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
