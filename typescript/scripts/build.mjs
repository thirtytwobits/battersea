import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
for (const name of fs.readdirSync(path.join(root, "packages")).sort((a, b) => a === "flow" ? -1 : b === "flow" ? 1 : a.localeCompare(b))) {
  const dir = path.join(root, "packages", name);
  if (!fs.existsSync(path.join(dir, "package.json"))) continue;
  fs.rmSync(path.join(dir, "dist"), { recursive: true, force: true });
  execFileSync(
    process.execPath,
    [path.join(root, "node_modules/typescript/bin/tsc"), "-p", dir],
    { stdio: "inherit" },
  );
  for (const name of ["LICENSE", "NOTICE"]) {
    fs.copyFileSync(path.join(root, "..", name), path.join(dir, name));
  }
}
