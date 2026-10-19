# Application tools

## Records

`battersea_tools::Tool` is the Model Context Protocol 2026-07-28 tool record, named by
`MCP_PROTOCOL_REVISION`. Its fields keep the protocol's spelling, `_meta` included.
`tools.schema.json` publishes it, and `@battersea/flow` carries its TypeScript types.

A tool's annotations are its side-effect class. Each hint is a claim of the tool's author.
`Tool::effect` reads every hint at its protocol default: an unannotated tool may change and
destroy, is not idempotent, and reaches outside its own domain.

## Profiles

A tool lists the agent profiles permitted to call it under
`_meta["io.github.thirtytwobits.battersea/profiles"]`, `PROFILES_META_KEY`. A tool that lists
none is permitted to none. The application names its profiles and chooses each run's profile.

## Registry

`ToolRegistry::register` admits a record Battersea can offer to a provider. It refuses with a
`ToolCode`:

| Record | Code |
|---|---|
| Name outside 1 to 128 ASCII letters, digits, `_`, `-` and `.` | `invalid_tool_definition` |
| No description | `invalid_tool_definition` |
| Input schema whose root is not an object schema | `invalid_tool_definition` |
| Profiles that are not distinct, non-blank names | `invalid_tool_definition` |
| Input schema that does not compile | `invalid_tool_schema` |
| Provider-facing name already registered | `duplicate_tool` |

A provider is offered each tool under its provider-facing name, `local_tool_name` of its own,
with its description exactly as written.
`ToolRegistry::for_profile` keeps the tools permitted to one profile. A call names its tool by
either name. The registry refuses a call to a tool it does not hold as `unregistered_tool`, and
arguments outside the tool's input schema as `invalid_tool_arguments`, before any executor runs.
The executor receives the call under the tool's own name.
