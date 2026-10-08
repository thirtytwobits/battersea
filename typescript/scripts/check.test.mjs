import assert from "node:assert/strict";
import test from "node:test";
import { importViolations, packageViolations } from "./check.mjs";
test("product imports are rejected across static, type, export and dynamic forms", () => {
  for (const source of [
    "import x from '@primrose/protocol';",
    "import type {X} from '@clerkenwell/client';",
    "export * from '@primrose/flow';",
    "await import('@primrose/wire');",
  ])
    assert.ok(importViolations(source).length);
  assert.deepEqual(
    importViolations(
      "// @primrose/example\nimport type {X} from './contract.js';",
    ),
    [],
  );
});
test("package dependencies cannot conceal a product through npm aliases", () => {
  const manifest = {
    name: "@battersea/flow",
    version: "2.3.4",
    dependencies: { innocent: "npm:@primrose/wire@1.0.0" },
  };
  const release = { version: manifest.version, npm: { [manifest.name]: [] } };
  assert.ok(packageViolations(manifest, release).length);
  delete manifest.dependencies;
  assert.deepEqual(packageViolations(manifest, release), []);
  manifest.version = "2.3.5";
  assert.ok(packageViolations(manifest, release).length);
});
