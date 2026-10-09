import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const temp = fs.mkdtempSync(
  path.join(os.tmpdir(), "battersea-editor-example-"),
);
const children = [];
const run = (command, args, cwd = temp) =>
  execFileSync(command, args, { cwd, stdio: "inherit" });
const start = (command, args, cwd = temp) => {
  const child = spawn(command, args, { cwd, stdio: "inherit" });
  children.push(child);
  return child;
};
async function ready(url, child) {
  const until = Date.now() + 30_000;
  while (Date.now() < until) {
    assert.equal(
      child.exitCode,
      null,
      `Server stopped before ${url} became ready`,
    );
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Server did not become ready: ${url}`);
}
try {
  const packs = JSON.parse(
    execFileSync(
      "npm",
      ["pack", "--workspaces", "--json", "--pack-destination", temp],
      { cwd: path.join(root, "typescript"), encoding: "utf8" },
    ),
  );
  const web = path.join(temp, "web");
  fs.cpSync(path.join(root, "examples/editor-server/web"), web, {
    recursive: true,
    filter: (source) =>
      !["node_modules", "dist", "test-artifacts", "package-lock.json"].includes(
        path.basename(source),
      ),
  });
  run(
    "npm",
    [
      "install",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
      ...packs.map((p) => path.join(temp, p.filename)),
    ],
    web,
  );
  run("npm", ["run", "build"], web);
  run(
    "npx",
    [
      "playwright",
      "install",
      ...(process.env.CI ? ["--with-deps"] : []),
      "chromium",
      "webkit",
    ],
    web,
  );
  const server =
    process.env.EXAMPLE_SERVER ??
    path.join(root, "target/debug/editor-server-example");
  const data = path.join(temp, "data");
  run(server, ["init", data]);
  const api = start(server, ["serve", data]);
  await ready("http://127.0.0.1:18180/api/flows", api);
  const vite = start(
    process.execPath,
    [
      path.join(web, "node_modules/vite/bin/vite.js"),
      "--host",
      "127.0.0.1",
      "--port",
      "18181",
      "--strictPort",
    ],
    web,
  );
  await ready("http://127.0.0.1:18181", vite);
  run("npm", ["test"], web);
  const activation = JSON.parse(
    execFileSync(
      server,
      ["activate", "example", "text", "A CLI-authored input"],
      { encoding: "utf8" },
    ),
  );
  const snapshot = JSON.parse(
    execFileSync(server, ["inspect", activation.id], { encoding: "utf8" }),
  );
  assert.equal(snapshot.id, activation.id);
  run(server, ["cancel", activation.id]);
  console.log("Installed editor, server and CLI share activation identities");
} finally {
  await Promise.all(
    children.map(
      (child) =>
        new Promise((resolve) => {
          if (child.exitCode !== null) return resolve();
          child.once("exit", resolve);
          child.kill("SIGTERM");
        }),
    ),
  );
  const screenshots = path.join(temp, "web/test-artifacts");
  if (fs.existsSync(screenshots)) fs.cpSync(screenshots, path.join(root, "target/editor-browser"), { recursive: true });
  fs.rmSync(temp, { recursive: true, force: true });
}
