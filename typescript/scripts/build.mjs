import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
for (const name of fs.readdirSync(path.join(root, "packages"))) {
  const dir = path.join(root, "packages", name);
  if (!fs.existsSync(path.join(dir, "package.json"))) continue;
  fs.rmSync(path.join(dir, "dist"), { recursive: true, force: true });
  execFileSync(
    process.execPath,
    [path.join(root, "node_modules/typescript/bin/tsc"), "-p", dir],
    { stdio: "inherit" },
  );
  fs.copyFileSync(path.join(root, "../NOTICE"), path.join(dir, "NOTICE"));
}
