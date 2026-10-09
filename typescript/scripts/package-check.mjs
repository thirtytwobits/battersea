import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const release = JSON.parse(fs.readFileSync(path.join(root, "../release.json")));
const temp = fs.mkdtempSync(path.join(os.tmpdir(), "battersea-package-"));
try {
  const archives = JSON.parse(
    execFileSync(
      "npm",
      ["pack", "--workspaces", "--json", "--pack-destination", temp],
      { cwd: root, encoding: "utf8" },
    ),
  );
  for (const pack of archives) {
    const paths = new Set(pack.files.map((file) => file.path));
    for (const required of release.npm[pack.name])
      assert.ok(paths.has(required), `${pack.name} omits ${required}`);
    const consumer = path.join(temp, pack.filename + "-consumer");
    fs.mkdirSync(consumer);
    fs.writeFileSync(
      path.join(consumer, "package.json"),
      JSON.stringify({ private: true, type: "module" }),
    );
    execFileSync(
      "npm",
      [
        "install",
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
        ...archives.map((entry) => path.join(temp, entry.filename)),
        "react@^18.3.1",
        "react-dom@^18.3.1",
        "@types/react@^18.3.1",
        "@types/react-dom@^18.3.1",
      ],
      { cwd: consumer, stdio: "inherit" },
    );
    const installed = path.join(consumer, "node_modules", pack.name);
    for (const name of ["LICENSE", "NOTICE"]) {
      assert.equal(
        fs.readFileSync(path.join(installed, name), "utf8"),
        fs.readFileSync(path.join(root, "..", name), "utf8"),
      );
    }
    assert.equal(
      JSON.parse(fs.readFileSync(path.join(installed, "package.json"))).license,
      release.license,
    );
    const source = `import * as flow from ${JSON.stringify(pack.name)};\nvoid flow;\n`;
    fs.writeFileSync(path.join(consumer, "index.mjs"), source);
    fs.writeFileSync(path.join(consumer, "index.ts"), source);
    execFileSync(process.execPath, ["index.mjs"], {
      cwd: consumer,
      stdio: "inherit",
    });
    execFileSync(
      process.execPath,
      [
        path.join(root, "node_modules/typescript/bin/tsc"),
        "--noEmit",
        "--strict",
        "--module",
        "NodeNext",
        "--target",
        "ES2022",
        "index.ts",
      ],
      { cwd: consumer, stdio: "inherit" },
    );
  }
} finally {
  fs.rmSync(temp, { recursive: true, force: true });
}
