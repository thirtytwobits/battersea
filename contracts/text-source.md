# Text source and code-defined nodes

Status: M6.6 contract. Text syntax version 1 carries flow document version 2.

```text
flow 1 "example" {
  version = 2;
  execution = {
    "source_order": ["source"],
    "limits": {
      "pending_events": 4096, "retained_bytes": 67108864, "node_retained_bytes": 16777216,
      "provider_queue": {"items": 32, "bytes": 1048576, "max_event_bytes": 262144, "policy": "backpressure"}
    }
  };
  title = "Example";
  node "source" "Source" {"instance_name": "Source", "parameter_values": {}}
  node "sink" "Sink" {"instance_name": "Sink"}
  edge "delivery" "source"."text" -> "sink"."text" {"order": 0}
}
```

Identifiers are JSON strings. Token edges use `->`; signal edges use `~>`. Document attributes
use `name = JSON;`. Node and edge attributes are JSON objects containing the remaining document
fields. Whitespace separates tokens; `//` comments are allowed between tokens. JSON values obey
JSON escaping and number rules. Comments and source formatting are not persisted.

Parsing and exporting preserve all document fields, node/edge array order, edge order, source
order, declared parameter values, port names/order/parameters, automation targets, queue limits,
metadata and client layout. Object key order is insignificant. Export is deterministic.

Syntax and document versions are explicit and independent. Unsupported versions fail without
conversion. Duplicate attributes, JSON keys, node IDs and edge IDs fail instead of overwriting.
Attributes cannot override identities or endpoints declared by the syntax. Unknown fields and
invalid typed payloads fail. Diagnostics carry one-based line and column. Parsing is pure;
catalogue/graph validation and explicit document upgrades are separate operations.

`NodeDefinition` supplies one inline manifest through a Rust derive. Definition construction uses
`parse_definition_manifest` with the host registry. Catalogue assembly then applies the same
validation and built-in ports as file manifests. Derived metadata cannot implement execution.
`RegistryBuilder::register_defined` rejects mismatched handler IDs and duplicates without
registering a handler. Rust generic parameters and an explicit renamed flow-crate path are supported.
