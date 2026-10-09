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
      ".battersea-editor__workspace, .react-flow__node, .flow-studio-node-frame, .authoring-graph-handle, .flow-studio-node-port-label, .flow-studio-node-port-select, aside, [data-phase], .battersea-execution-settings[open], .battersea-execution-settings[open] input, .battersea-execution-settings[open] select, .battersea-execution-settings[open] [data-execution-state], .battersea-execution-settings[open] [data-execution-mode], .battersea-execution-settings[open] [data-execution-policy]",
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
          pointerEvents: c.pointerEvents,
          transform: c.transform,
          left: c.left,
          right: c.right,
          top: c.top,
          bottom: c.bottom,
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
    const branchBaseline = await read(page);
    const knownNodes = new Set(branchBaseline.nodes.map(node => node.id));
    await page.getByRole("button", { name: "Output", exact: true }).click();
    await page.waitForFunction(count => document.querySelectorAll(".react-flow__node").length === count, knownNodes.size + 1);
    const branchNode = await page.locator(".react-flow__node").evaluateAll((nodes, known) => nodes.map(node => node.dataset.id).find(id => !known.includes(id)), [...knownNodes]);
    assert.ok(branchNode);
    const branchStyle = await page.locator(`.react-flow__node[data-id="${branchNode}"]`).getAttribute("style");
    await page.getByRole("button", { name: "Auto layout: apply Dagre", exact: true }).click();
    await page.waitForFunction(({id, before}) => document.querySelector(`.react-flow__node[data-id="${id}"]`)?.getAttribute("style") !== before, {id: branchNode, before: branchStyle});
    await page.locator(".react-flow__controls-fitview").click();
    await page.waitForFunction(() => document.querySelector('.react-flow__node[data-id="text"]').getBoundingClientRect().width > 100);
    await capture(page, name, "fanout-ready");
    const outputHandle = page.locator('.react-flow__node[data-id="text"] .react-flow__handle[data-handleid="output-0"]');
    const inputHandle = page.locator(`.react-flow__node[data-id="${branchNode}"] .react-flow__handle[data-handleid="input-0"]`);
    // Locator actions wait for fit-view animation to finish before hit testing.
    await outputHandle.hover();
    await inputHandle.hover();
    await outputHandle.hover();
    const to = await inputHandle.boundingBox();
    assert.ok(to);
    await page.mouse.down();
    await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 12 });
    await page.waitForFunction(id => document.querySelector(`.react-flow__node[data-id="${id}"] .react-flow__handle[data-handleid="input-0"]`)?.classList.contains("valid"), branchNode);
    await capture(page, name, "fanout-connecting");
    await page.mouse.up();
    await capture(page, name, "fanout-connected");
    await page.waitForFunction(count => document.querySelectorAll(".react-flow__edge").length === count, branchBaseline.edges.length + 1);
    await page.getByRole("button", { name: "Undo", exact: true }).click();
    await page.waitForFunction(count => document.querySelectorAll(".react-flow__edge").length === count, branchBaseline.edges.length);
    await page.getByRole("button", { name: "Redo", exact: true }).click();
    await page.waitForFunction(count => document.querySelectorAll(".react-flow__edge").length === count, branchBaseline.edges.length + 1);
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await page.waitForFunction(() => document.querySelector(".battersea-editor__toolbar output")?.textContent === "Saved");
    const branched = await read(page);
    assert.equal(branched.edges.length, branchBaseline.edges.length + 1);
    assert.deepEqual(branched.execution, branchBaseline.execution);
    await page.getByRole("button", { name: "Reload", exact: true }).click();
    await page.locator('.react-flow__node[data-id="text"]').click();
    await page.getByRole("textbox", { name: "Text", exact: true }).fill(`fan-out ${name}`);
    await page.getByRole("button", { name: "Activate node", exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[aria-label="Activation status"]')?.textContent?.endsWith("succeeded"));
    const branchRun = (await page.getByLabel("Activation status").textContent()).split(":")[0];
    const branchEvents = await (await page.request.get(`${origin}/api/activations/${branchRun}`)).json();
    assert.equal(branchEvents.events.filter(event => event.phase === "flow.token.receive").length, branched.edges.length);
    await capture(page, name, "fanout");
    await page.getByText("Execution settings", {exact: true}).click();
    assert.equal(await page.locator('[data-execution-mode="final_value"]').count(), 1);
    assert.equal(await page.getByRole("combobox", {name: "Overflow policy", exact: true}).count(), 0);
    await page.getByRole("spinbutton", {name: "Connection order", exact: true}).scrollIntoViewIfNeeded();
    await capture(page, name, "execution-final");
    const policyFlow = structuredClone(before);
    policyFlow.flow_key = `policy-${name}`;
    policyFlow.title = `Execution policy ${name}`;
    policyFlow.nodes = [
      {...policyFlow.nodes.find(node => node.id === "text"), definition_name: "TextStream"},
      {...policyFlow.nodes.find(node => node.id === "output"), definition_name: "StreamOutput"},
    ];
    policyFlow.nodes.push({...structuredClone(policyFlow.nodes[0]), id: "secondary", instance_name: "Second source"});
    policyFlow.execution.source_order = ["text", "secondary"];
    policyFlow.edges = [{...policyFlow.edges[0], queue: structuredClone(policyFlow.execution.limits.provider_queue)}];
    assert((await page.request.put(`${origin}/api/flows`, {data: policyFlow})).ok());
    await page.reload();
    await page.getByRole("combobox", {name: "Flow", exact: true}).selectOption(policyFlow.flow_key);
    await page.getByText("Execution settings", {exact: true}).click();
    const sourceOrder = () => page.locator('[aria-label="Source priority"] li').evaluateAll(items => items.map(item => item.dataset.sourceId));
    assert.deepEqual(await sourceOrder(), policyFlow.execution.source_order);
    await page.getByRole("button", {name: "Move Text source (text) later", exact: true}).click();
    assert.deepEqual(await sourceOrder(), [...policyFlow.execution.source_order].reverse());
    await page.getByRole("button", {name: "Undo", exact: true}).click();
    assert.deepEqual(await sourceOrder(), policyFlow.execution.source_order);
    await page.getByRole("button", {name: "Redo", exact: true}).click();
    await capture(page, name, "execution-priority");
    const policyRead = async () => (await page.request.get(`${origin}/api/flows/${policyFlow.flow_key}`)).json();
    const pending = page.getByRole("spinbutton", {name: "Pending events", exact: true});
    await pending.fill("1.5");
    await page.getByRole("button", {name: "Apply activation limits", exact: true}).click();
    assert(await page.locator('[data-execution-state="invalid"]').isVisible());
    assert.deepEqual(await policyRead(), policyFlow, "Rejected drafts must not save");
    await capture(page, name, "execution-invalid");
    const authoredLimits = structuredClone(policyFlow.execution.limits);
    authoredLimits.pending_events *= 2;
    authoredLimits.node_retained_bytes /= 2;
    authoredLimits.provider_queue.items /= 2;
    await pending.fill(String(authoredLimits.pending_events));
    await page.getByRole("spinbutton", {name: "Node retained bytes", exact: true}).fill(String(authoredLimits.node_retained_bytes));
    await page.getByRole("spinbutton", {name: "Provider queue items", exact: true}).fill(String(authoredLimits.provider_queue.items));
    await page.getByRole("button", {name: "Apply activation limits", exact: true}).click();
    assert.equal(await page.locator('[data-execution-state="invalid"]').count(), 0);
    await page.getByRole("button", {name: "Undo", exact: true}).click();
    assert.equal(await pending.inputValue(), String(policyFlow.execution.limits.pending_events));
    await page.getByRole("button", {name: "Redo", exact: true}).click();
    assert.equal(await pending.inputValue(), String(authoredLimits.pending_events));
    await pending.scrollIntoViewIfNeeded();
    await capture(page, name, "execution-limits");
    assert.equal(await page.locator('[data-execution-mode="stream"]').count(), 1);
    const authoredQueue = {items: 5, bytes: 500, max_event_bytes: 200, policy: "drop_oldest"};
    const authoredOrder = 2;
    await page.getByRole("spinbutton", {name: "Connection order", exact: true}).fill(String(authoredOrder));
    for (const [label, value] of [["Queue items", authoredQueue.items], ["Queue bytes", authoredQueue.bytes], ["Maximum event bytes", authoredQueue.max_event_bytes]]) {
      await page.getByRole("spinbutton", {name: label, exact: true}).fill(String(value));
    }
    await page.getByRole("combobox", {name: "Overflow policy", exact: true}).selectOption(authoredQueue.policy);
    await page.getByRole("button", {name: "Apply connection settings", exact: true}).click();
    await capture(page, name, "execution-lossy");
    await page.getByRole("button", {name: "Undo", exact: true}).click();
    assert.equal(await page.getByRole("combobox", {name: "Overflow policy", exact: true}).inputValue(), policyFlow.edges[0].queue.policy);
    await page.getByRole("combobox", {name: "Overflow policy", exact: true}).scrollIntoViewIfNeeded();
    await capture(page, name, "execution-backpressure");
    await page.getByRole("button", {name: "Redo", exact: true}).click();
    await page.getByRole("button", {name: "Save", exact: true}).click();
    await page.waitForFunction(() => document.querySelector(".battersea-editor__toolbar output")?.textContent === "Saved");
    const policySaved = await policyRead();
    assert.deepEqual(policySaved.execution.source_order, [...policyFlow.execution.source_order].reverse());
    assert.deepEqual(policySaved.execution.limits, authoredLimits);
    assert.deepEqual(policySaved.edges[0].queue, authoredQueue);
    assert.equal(policySaved.edges[0].order, authoredOrder);
    await page.getByRole("button", {name: "Reload", exact: true}).click();
    assert.deepEqual(await policyRead(), policySaved);
    await page.getByText("Execution settings", {exact: true}).click();
    assert.equal(await pending.inputValue(), String(authoredLimits.pending_events));
    assert.equal(await page.getByRole("combobox", {name: "Overflow policy", exact: true}).inputValue(), authoredQueue.policy);
    await pending.fill("0");
    await page.getByRole("button", {name: "Reload", exact: true}).click();
    await page.getByText("Execution settings", {exact: true}).click();
    assert.equal(await pending.inputValue(), String(authoredLimits.pending_events), "Reload discards unapplied settings");
    await page.getByText("Execution settings", {exact: true}).click();
    await page.locator('.react-flow__node[data-id="text"]').click();
    await page.getByRole("textbox", {name: "Text", exact: true}).fill(`stream ${name}`);
    await page.getByRole("button", {name: "Activate node", exact: true}).click();
    await page.waitForFunction(() => document.querySelector('[aria-label="Activation status"]')?.textContent?.endsWith("succeeded"));
    console.log(`${name}: execution priority, limit rejection, undo/redo, streaming queues, save/reload and activation passed`);
    assert.deepEqual(errors, []);
    console.log(
      `${name}: load, read purity, edit, undo, save/reload, layout, activate, diagnostics, cancel and fan-out edit/undo/save/reload/execute passed`,
    );
  } finally {
    await browser.close();
  }
}
