# Your first flow

From a source checkout, run the independent catalogue example:

```sh
cargo run --locked -p catalogue-example
```

It registers an application handler, a nominal token schema, and a parameter schema; builds
a catalogue; validates a version 2 flow; and prints canonical JSON. It also checks that
opaque editor layout and application metadata survive a save/load round trip.

The complete, compiled example is included below so it stays aligned with the released API:

```rust,ignore
{{#include examples/catalogue.rs}}
```

Validation is separate from execution. An application must register a handler implementation
before it can run a node. [Implement a node](custom-nodes.md) next, or read the
[text-source contract](contracts/text-source.md) for the equivalent authored flow syntax.

Validation issues have stable codes. Match those codes when presenting application diagnostics;
keep messages available for people to read.
