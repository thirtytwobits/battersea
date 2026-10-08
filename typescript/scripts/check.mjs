import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export function importViolations(source, filename = "input.ts") {
  const errors = [];
  const visit = (node) => {
    if (
      ts.isStringLiteralLike(node) &&
      /^@(primrose|clerkenwell)\//.test(node.text)
    ) {
      errors.push(`${filename}: product dependency ${node.text}`);
    }
    ts.forEachChild(node, visit);
  };
  visit(ts.createSourceFile(filename, source, ts.ScriptTarget.Latest, true));
  return errors;
}
export function packageViolations(manifest, release) {
  const errors = [];
  if (
    manifest.version !== release.version ||
    !Object.hasOwn(release.npm, manifest.name)
  )
    errors.push(`${manifest.name}: release version/inventory mismatch`);
  for (const field of [
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
  ]) {
    for (const [name, spec] of Object.entries(manifest[field] ?? {})) {
      if (
        /^@(primrose|clerkenwell)\//.test(name) ||
        /^npm:@(primrose|clerkenwell)\//.test(spec)
      )
        errors.push(`${manifest.name}: product dependency ${name}`);
      if (/^(file|link):/.test(spec))
        errors.push(`${manifest.name}: local path dependency ${name}`);
    }
  }
  return errors;
}
function files(dir) {
  return fs
    .readdirSync(dir, { withFileTypes: true })
    .flatMap((e) =>
      e.isDirectory()
        ? files(path.join(dir, e.name))
        : [path.join(dir, e.name)],
    )
    .sort();
}
function main() {
  const release = JSON.parse(
    fs.readFileSync(path.join(root, "../release.json")),
  );
  const errors = [];
  const lock = JSON.parse(
    fs.readFileSync(path.join(root, "package-lock.json")),
  );
  for (const [location, entry] of Object.entries(lock.packages ?? {})) {
    const name =
      entry.name ??
      location.match(/(?:^|\/)node_modules\/(@[^/]+\/[^/]+)$/)?.[1];
    if (/^@(primrose|clerkenwell)\//.test(name ?? ""))
      errors.push(`${location}: transitive product dependency`);
  }
  const found = new Set();
  for (const name of fs.readdirSync(path.join(root, "packages"))) {
    const dir = path.join(root, "packages", name);
    if (!fs.existsSync(path.join(dir, "package.json"))) continue;
    const manifest = JSON.parse(
      fs.readFileSync(path.join(dir, "package.json")),
    );
    found.add(manifest.name);
    errors.push(...packageViolations(manifest, release));
    for (const file of files(path.join(dir, "src")))
      errors.push(...importViolations(fs.readFileSync(file, "utf8"), file));
    const declarations = files(path.join(dir, "dist"))
      .filter((f) => f.endsWith(".d.ts"))
      .map(
        (f) =>
          `// ${path.relative(path.join(dir, "dist"), f).replaceAll(path.sep, "/")}\n${fs.readFileSync(f, "utf8")}`,
      )
      .join("\n");
    const snapshot = path.join(root, "../api", `${name}.d.ts`);
    if (process.argv.includes("--update-api"))
      fs.writeFileSync(snapshot, declarations);
    else if (fs.readFileSync(snapshot, "utf8") !== declarations)
      errors.push(`${manifest.name}: public API snapshot is stale`);
  }
  for (const name of Object.keys(release.npm))
    if (!found.has(name)) errors.push(`${name}: release package is absent`);
  if (errors.length) throw new Error(errors.join("\n"));
}
if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
)
  main();
