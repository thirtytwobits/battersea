import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { TOOL_PROFILES_META_KEY, toolProfiles } from '../packages/flow/dist/index.js';

test('a tool reads back the profiles it lists, and lists none without an entry', () => {
  const profiles = ['author', 'player'];
  assert.deepEqual(toolProfiles({ _meta: { [TOOL_PROFILES_META_KEY]: profiles } }), profiles);
  assert.deepEqual(toolProfiles({}), []);
  assert.deepEqual(toolProfiles({ _meta: { 'com.example/note': true } }), []);
});

test('a profiles entry that is not a list of names reads as undefined', () => {
  for (const entry of ['author', [1], [null], { author: true }]) {
    assert.equal(toolProfiles({ _meta: { [TOOL_PROFILES_META_KEY]: entry } }), undefined, JSON.stringify(entry));
  }
});

test('the published tool schema names the profiles key the package reads', async () => {
  const schema = JSON.parse(await readFile(new URL('../packages/flow/schemas/tools.schema.json', import.meta.url), 'utf8'));
  assert.ok(schema.$defs.Tool.properties._meta);
  assert.ok(schema.$defs.Tool.required.includes('inputSchema'));
  assert.match(TOOL_PROFILES_META_KEY, /^[a-z][a-z0-9-]*(\.[a-z][a-z0-9-]*)+\/[a-z]+$/);
});
