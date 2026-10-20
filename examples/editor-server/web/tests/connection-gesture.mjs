/** Copyright (c) Scott A Dixon */
import assert from "node:assert/strict";

export async function moveConnectionPointer(page, source, target) {
  await source.hover();
  await page.mouse.down();
  // Resolve the target after mouse-down, then again after the drag starts:
  // viewport and node movement can change the handle's screen position.
  await target.hover();
  await target.hover();
}

export async function testConnectionGesture(page, origin) {
  await page.goto(`${origin}/tests/node-measurement-recovery.html`);
  await page.waitForFunction(() => document.querySelectorAll(".react-flow__edge").length === 1);
  await page.getByRole("checkbox", { name: "Move branch when connecting" }).check();
  const source = page.locator('.react-flow__node[data-id="source"] .react-flow__handle[data-handleid="output-0"]');
  const target = page.locator('.react-flow__node[data-id="branch"] .react-flow__handle[data-handleid="input-0"]');
  const before = await target.boundingBox();
  assert.ok(before);
  try {
    await moveConnectionPointer(page, source, target);
    await target.and(page.locator(".valid")).waitFor({ timeout: 5000 });
    const after = await target.boundingBox();
    assert.ok(after);
    assert(after.x > before.x + 100 && after.y > before.y + 100,
      "The target must move away from the coordinates sampled before the drag");
  } finally {
    await page.mouse.up();
  }
  await page.waitForFunction(() => document.querySelectorAll(".react-flow__edge").length === 2);
  assert.equal(await page.locator('.react-flow__edge[data-id*="sourceoutput-0-branchinput-0"]').count(), 1);
}
