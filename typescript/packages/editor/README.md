# Battersea editor

Unstyled React components and editing behaviour for the released flow contract. `FlowEditor`
composes the palette, canvas, inspector, document lifecycle, undo and activation diagnostics.
Hosts supply `FlowEditorPorts`, lifecycle/interaction services, presentation and parameter renderers.
`graph` exports the shared canvas, handles, editable edges and gesture primitives for other graph editors.

Import React Flow's base stylesheet in the host. The kit imports no CSS. Its semantic classes
and presentation slots allow the host to supply its design system. Layout computation is an
injected function; the default lazily loads Dagre. Primrose supplies its data worker.

A parameter renderer returns `undefined` to delegate to the primitive control or `null` to hide
its field. Custom source resolution and storage remain host responsibilities. Document inspection
preserves parameter values and does not infer upgrades from their shape.

The editor-server example connects an installed package to application-owned HTTP endpoints.
Component stories cover nodes, edges, palette and inspector. Dependency notices are packaged
in `THIRD_PARTY_NOTICES.md`.
