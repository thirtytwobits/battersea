import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { checkDocumentationEntryPoints } from "./docs-entrypoints.mjs";

test("documentation covers public root, wildcard subpaths and TSX without documenting private modules", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "battersea-docs-"));
  try {
    fs.mkdirSync(path.join(directory, "src/components"), { recursive: true });
    for (const name of ["index.ts", "internal.ts", "components/widget.tsx"]) {
      fs.writeFileSync(path.join(directory, "src", name), "");
    }
    fs.writeFileSync(path.join(directory, "package.json"), JSON.stringify({ name: "fixture", exports: {
      ".": { types: "./dist/index.d.ts" },
      "./components/*": { types: "./dist/components/*.d.ts" },
      "./schema": "./schema.json",
    } }));
    const configure = (entryPoints) => fs.writeFileSync(path.join(directory, "typedoc.json"), JSON.stringify({ entryPoints }));
    configure(["src/index.ts", "src/components/*.tsx"]);
    checkDocumentationEntryPoints(directory);
    configure(["src/index.ts"]);
    assert.throws(() => checkDocumentationEntryPoints(directory), /do not match/);
    configure(["src/*.ts", "src/components/*.tsx"]);
    assert.throws(() => checkDocumentationEntryPoints(directory), /do not match/);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});
