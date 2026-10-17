/** Copyright (c) Scott A Dixon */
import assert from "node:assert/strict";

export async function testNodeMeasurementRecovery(page, origin) {
  await page.addInitScript(() => {
    const NativeResizeObserver = window.ResizeObserver;
    window.ResizeObserver = class extends NativeResizeObserver {
      constructor(callback) {
        super((entries, observer) => {
          // Hold node notifications to reproduce missing handle bounds deterministically.
          const delivered = entries.filter(entry =>
            !window.holdNodeResizeNotifications || !entry.target.matches(".react-flow__node"));
          if (delivered.length) callback(delivered, observer);
        });
      }
    };
  });
  await page.goto(`${origin}/tests/node-measurement-recovery.html`);
  await page.waitForFunction(() => document.querySelectorAll(".react-flow__edge").length === 1);
  for (let attempt = 0; attempt < 3; attempt++) {
    await page.getByRole("button", { name: "Restore measured nodes" }).click();
    await page.waitForFunction(() =>
      document.querySelectorAll(".react-flow__edge").length === 1
      && document.querySelectorAll(".react-flow__node").length === 3
      && [...document.querySelectorAll(".react-flow__node")].every(node =>
        getComputedStyle(node).visibility === "visible"), null, { timeout: 5000 });
  }
  const source = page.locator('.react-flow__node[data-id="source"] .react-flow__handle[data-handleid="output-0"]');
  const target = page.locator('.react-flow__node[data-id="branch"] .react-flow__handle[data-handleid="input-0"]');
  await source.hover();
  const to = await target.boundingBox();
  assert.ok(to);
  await page.mouse.down();
  await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 12 });
  await page.waitForFunction(() =>
    document.querySelector('.react-flow__node[data-id="branch"] .react-flow__handle[data-handleid="input-0"]')?.classList.contains("valid"));
  await page.mouse.up();
  await page.waitForFunction(() => document.querySelectorAll(".react-flow__edge").length === 2);
}
