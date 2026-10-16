import fs from "node:fs";
import path from "node:path";

function matches(file, pattern) {
  const expression = pattern.split("*")
    .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
    .join("[^/]*");
  return new RegExp(`^${expression}$`).test(file);
}

/** Refuse a reference which silently omits public subpaths or exposes extra entry points. */
export function checkDocumentationEntryPoints(directory) {
  const manifest = JSON.parse(fs.readFileSync(path.join(directory, "package.json"), "utf8"));
  const config = JSON.parse(fs.readFileSync(path.join(directory, "typedoc.json"), "utf8"));
  const files = fs.readdirSync(path.join(directory, "src"), { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile() && /\.tsx?$/.test(entry.name))
    .map((entry) => path.relative(directory, path.join(entry.parentPath, entry.name)).split(path.sep).join("/"));
  const exports = Object.values(manifest.exports).flatMap((entry) => {
    const types = entry?.types;
    if (!types) return [];
    return [types.replace(/^\.\/dist\//, "src/").replace(/\.d\.ts$/, ".ts")];
  });
  const exported = files.filter((file) => exports.some((pattern) =>
    matches(file, pattern) || matches(file, `${pattern}x`))).sort();
  const documented = files.filter((file) => config.entryPoints.some((pattern) => matches(file, pattern))).sort();
  if (!exported.length || JSON.stringify(exported) !== JSON.stringify(documented)) {
    throw new Error(`TypeDoc entry points do not match the public exports of ${manifest.name}\nExported: ${exported}\nDocumented: ${documented}`);
  }
}
