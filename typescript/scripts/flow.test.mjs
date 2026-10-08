import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { tokenConnectionCompatible } from '../packages/flow/dist/index.js';
import { flows } from '../packages/flow/dist/fixtures.js';
const read = (name) => JSON.parse(readFileSync(new URL(`../packages/flow/fixtures/${name}`, import.meta.url)));

test('the editor and Rust agree on every declared nominal token connection', () => {
  for (const c of read('token-type-compatibility.json').cases) {
    assert.equal(tokenConnectionCompatible(c.source_token_type, c.source_node_input_accepted, c.target_accepted), c.expected, c.name);
  }
});

test('Rust documents satisfy the TypeScript contract and preserve opaque editor state', () => {
  const serialised = JSON.parse(JSON.stringify(flows));
  assert.deepEqual(serialised, read('flows.json'));
  for (const flow of serialised) {
    const originalLayout = structuredClone(flow.layout);
    const originalMetadata = structuredClone(flow.metadata);
    flow.title = 'Edited through a TypeScript consumer';
    const roundTrip = JSON.parse(JSON.stringify(flow));
    assert.deepEqual(roundTrip.layout, originalLayout);
    assert.deepEqual(roundTrip.metadata, originalMetadata);
  }
});
