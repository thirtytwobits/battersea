# Integrate the editor

`@battersea/editor` provides `FlowEditor` and a separate `@battersea/editor/graph` entry point.
The package supplies behaviour and semantic classes. Your application supplies presentation,
document storage, catalogue loading, validation, activation, and parameter renderers.

Import React Flow's base stylesheet in the host:

```ts
import "@xyflow/react/dist/style.css";
import { FlowEditor } from "@battersea/editor";
```

Implement the `FlowEditorPorts` interface and supply the lifecycle, interaction, and presentation
services declared by `FlowEditorProps`. Use the [TypeScript reference](../typescript/index.html)
for the complete signatures, including the package's public subpath exports.

Parameter renderers return `undefined` to delegate to the primitive control or `null` to hide
a field. Layout is an injected function; the default loads Dagre on demand. The package does
not import application CSS or own application storage.

Connections require compatible token types and matching consumption modes. Inspection and
validation preserve document fields and do not infer upgrades. See the [editor package guide](editor.md)
and the [execution contract](contracts/execution.md).

For a complete application, follow the [editor-server example](editor-server.md). It connects
installed packages to application-owned HTTP endpoints and uses the same activation endpoints
from its CLI.
