import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { checkDocumentationEntryPoints } from "./docs-entrypoints.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const release = JSON.parse(fs.readFileSync(path.join(root, "release.json"), "utf8"));
for (const name of ["flow", "editor"]) {
  checkDocumentationEntryPoints(path.join(root, "typescript/packages", name));
}
const out = path.join(root, "target/documentation/site", `v${release.version}`, "typescript");
const options = path.join(root, "target/documentation/typedoc-options.json");
fs.mkdirSync(path.dirname(options), { recursive: true });
fs.writeFileSync(options, JSON.stringify({
  extends: path.join(root, "typescript/typedoc.json"),
  navigationLinks: {
    Handbook: `/battersea/v${release.version}/guide/index.html`,
    "Rust API": `/battersea/v${release.version}/rust/index.html`,
    Versions: "/battersea/versions/index.html",
  },
}));
fs.rmSync(out, { recursive: true, force: true });
execFileSync(process.execPath, [
  path.join(root, "typescript/node_modules/typedoc/bin/typedoc"),
  "--options", options, "--out", out,
], { cwd: path.join(root, "typescript"), stdio: "inherit" });
