import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { chromium, webkit } from "playwright";
const origin = process.env.EDITOR_URL ?? "http://127.0.0.1:18181";
const out = process.env.EDITOR_ARTIFACTS ?? "test-artifacts";
await fs.mkdir(out, { recursive: true });
async function read(page) {
  const response = await page.request.get(`${origin}/api/flows/example`);
  assert(response.ok());
  return response.json();
}
async function capture(page, name, state) {
  const styles = await page
    .locator(
      ".battersea-editor__workspace, .react-flow__node, .flow-studio-node-frame, .authoring-graph-handle, aside, [data-phase]",
    )
    .evaluateAll((elements) =>
      elements.map((el) => {
        const c = getComputedStyle(el),
          r = el.getBoundingClientRect();
        return {
          class: el.className,
          data: { ...el.dataset },
          colour: c.color,
          background: c.backgroundColor,
          font: c.fontFamily,
          size: c.fontSize,
          width: r.width,
          height: r.height,
          display: c.display,
          opacity: c.opacity,
        };
      }),
    );
  assert(styles.every((s) => s.width > 0 && s.height > 0));
  await fs.writeFile(
    `${out}/${name}-${state}.json`,
    JSON.stringify(styles, null, 2),
  );
  await page.screenshot({ path: `${out}/${name}-${state}.png` });
}
for (const [name, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch({ headless: true });
  try {
    const page = await browser.newPage({
      viewport: { width: 1440, height: 1000 },
    });
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    page.on("console", (m) => {
      if (m.type() === "error") errors.push(m.text());
    });
    page.on("dialog", (dialog) => dialog.accept());
    await page.goto(origin);
    const fixture = await read(page);
    fixture.layout = {
      ...fixture.layout,
      flow_builder_v1: {
        canvas: {
          nodes: {
            text: { position: { x: 0, y: 0 } },
            output: { position: { x: 5000, y: 1000 } },
          },
        },
      },
    };
    assert(
      (await page.request.put(`${origin}/api/flows`, { data: fixture })).ok(),
    );
    const before = await read(page);
    await page
      .getByRole("combobox", { name: "Flow", exact: true })
      .selectOption("example");
    await page.waitForFunction(
      () =>
        document.querySelector(".react-flow__viewport") &&
        getComputedStyle(document.querySelector(".react-flow__viewport"))
          .opacity === "1",
    );
    assert.deepEqual(await read(page), before, "Loading must not save");
    const bounds = await page.locator(".react-flow").boundingBox();
    for (const node of await page.locator(".react-flow__node").all()) {
      const rect = await node.boundingBox();
      assert(
        rect.x >= bounds.x &&
          rect.y >= bounds.y &&
          rect.x + rect.width <= bounds.x + bounds.width &&
          rect.y + rect.height <= bounds.y + bounds.height,
        "Initial framing must contain every node",
      );
    }
    await page.locator('.react-flow__node[data-id="text"]').click();
    await capture(page, name, "selected");
    assert.equal(
      await page
        .getByRole("textbox", { name: "Text", exact: true })
        .getAttribute("data-parameter-renderer"),
      "application",
    );
    const delay = page.getByRole("combobox", { name: "Delay Ms" });
    await delay.selectOption("1000");
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await page.waitForFunction(
      () =>
        document.querySelector(".battersea-editor__toolbar output")
          ?.textContent === "Saved",
    );
    let saved = await read(page);
    assert.equal(
      saved.nodes.find((n) => n.id === "text").parameter_values.delay_ms,
      1000,
    );
    await delay.selectOption("2000");
    await page.getByRole("button", { name: "Undo", exact: true }).click();
    await page.locator('.react-flow__node[data-id="text"]').click();
    assert.equal(await delay.inputValue(), "1000");
    const box = await page
      .locator(
        '.react-flow__node[data-id="text"] .flow-studio-node-shell__title',
      )
      .boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 100, {
      steps: 8,
    });
    await page.mouse.up();
    const previousPosition = await page
      .locator('.react-flow__node[data-id="text"]')
      .getAttribute("style");
    await page
      .getByRole("button", { name: "Auto layout: apply Dagre", exact: true })
      .click();
    await page.waitForFunction(
      (before) =>
        document
          .querySelector('.react-flow__node[data-id="text"]')
          ?.getAttribute("style") !== before,
      previousPosition,
    );
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await page.waitForFunction(
      () =>
        document.querySelector(".battersea-editor__toolbar output")
          ?.textContent === "Saved",
    );
    saved = await read(page);
    assert.ok(saved.layout?.flow_builder_v1);
    const layout = structuredClone(saved.layout);
    await page.getByRole("button", { name: "Reload", exact: true }).click();
    await page.locator('.react-flow__node[data-id="text"]').click();
    assert.deepEqual((await read(page)).layout, layout);
    const input = `authored ${name} text`;
    await page.getByRole("textbox", { name: "Text", exact: true }).fill(input);
    await page
      .getByRole("button", { name: "Activate node", exact: true })
      .click();
    await page.waitForFunction(() =>
      document
        .querySelector('[aria-label="Activation status"]')
        ?.textContent?.endsWith("succeeded"),
    );
    const status = await page.getByLabel("Activation status").textContent();
    const id = status.split(":")[0];
    const snapshot = await (
      await page.request.get(`${origin}/api/activations/${id}`)
    ).json();
    assert.equal(snapshot.id, id);
    assert.equal(snapshot.status, "succeeded");
    assert(snapshot.events.length > 1);
    assert(snapshot.events.every((e) => e.activation_id === id));
    await capture(page, name, "succeeded");
    await page
      .getByRole("button", { name: "Activate node", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Cancel activation", exact: true })
      .click();
    await page.waitForFunction(() =>
      document
        .querySelector('[aria-label="Activation status"]')
        ?.textContent?.endsWith("cancelled"),
    );
    await capture(page, name, "cancelled");
    assert.deepEqual(errors, []);
    console.log(
      `${name}: load, read purity, edit, undo, save/reload, layout, activate, diagnostics and cancel passed`,
    );
  } finally {
    await browser.close();
  }
}
