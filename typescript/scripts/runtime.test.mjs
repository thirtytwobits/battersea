import test from 'node:test';
import assert from 'node:assert/strict';
import { applyRuntimeDelta, applyRuntimeUpdate, RuntimeResyncRequired } from '../packages/flow/dist/index.js';
const cursor = n => ({ epoch: 'process', revision: String(n) });
const record = id => ({ id, context: {activation_id: 'run',flow_key:'flow',node_id:null,session_id:null}, started_at_ms:1,updated_at_ms:2,state:{kind:'activation',status:'succeeded'} });

test('a client applies contiguous updates and retention removals without mutating previous snapshots', () => {
  const original = {cursor:cursor(0),records:[]};
  const first = {base:cursor(0),cursor:cursor(1),changes:[{kind:'upsert',record:record('first')}]};
  const second = {base:cursor(1),cursor:cursor(2),changes:[{kind:'remove',id:'first'},{kind:'upsert',record:record('second')}]};
  const actual = applyRuntimeUpdate(original,{kind:'deltas',deltas:[first,second],cursor:cursor(2)});
  assert.deepEqual(actual.records,[record('second')]); assert.deepEqual(original.records,[]);
  assert.throws(() => applyRuntimeDelta(original,second),RuntimeResyncRequired);
  assert.throws(() => applyRuntimeDelta(actual,second),RuntimeResyncRequired);
});
test('epoch changes and cursor gaps require resync even when revisions exceed number precision', () => {
  const big = 9007199254740993n;
  const snapshot = {cursor:cursor(big),records:[]};
  const delta = {base:cursor(big),cursor:cursor(big+1n),changes:[]};
  assert.deepEqual(applyRuntimeDelta(snapshot,delta).cursor,delta.cursor);
  assert.throws(() => applyRuntimeDelta(snapshot,{...delta,base:{...delta.base,epoch:'other'}}),RuntimeResyncRequired);
  assert.throws(() => applyRuntimeUpdate(snapshot,{kind:'deltas',deltas:[],cursor:cursor(big+1n)}),RuntimeResyncRequired);
  const restarted = {cursor:{epoch:'new-process',revision:'0'},records:[]};
  assert.deepEqual(applyRuntimeUpdate(snapshot,{kind:'resync',snapshot:restarted}),restarted);
});
