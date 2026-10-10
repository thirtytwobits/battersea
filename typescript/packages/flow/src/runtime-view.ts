import type { Delta, Snapshot } from "./runtime-contract.js";

export class RuntimeResyncRequired extends Error {
  constructor() { super("Runtime cursor requires a snapshot"); }
}
const revision = (value: string): bigint => {
  if (!/^(0|[1-9][0-9]*)$/.test(value)) throw new RuntimeResyncRequired();
  const result = BigInt(value);
  if (result > 18446744073709551615n) throw new RuntimeResyncRequired();
  return result;
};
/** Returns a replacement snapshot; stale, duplicate and missing deltas cannot mutate it. */
export function applyRuntimeDelta(snapshot: Snapshot, delta: Delta): Snapshot {
  if (snapshot.cursor.epoch !== delta.base.epoch || delta.cursor.epoch !== delta.base.epoch
    || snapshot.cursor.revision !== delta.base.revision
    || revision(delta.cursor.revision) !== revision(delta.base.revision) + 1n) {
    throw new RuntimeResyncRequired();
  }
  const records = new Map(snapshot.records.map(record => [record.id, record]));
  for (const change of delta.changes) {
    if (change.kind === "remove") records.delete(change.id);
    else records.set(change.record.id, change.record);
  }
  return { cursor: delta.cursor, records: [...records.values()].sort((a,b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0) };
}

export function applyRuntimeUpdate(snapshot: Snapshot | null, update: import("./runtime-contract.js").Update): Snapshot {
  if (update.kind === "resync") return update.snapshot;
  if (!snapshot) throw new RuntimeResyncRequired();
  let next = snapshot;
  for (const delta of update.deltas) next = applyRuntimeDelta(next, delta);
  if (next.cursor.epoch !== update.cursor.epoch || next.cursor.revision !== update.cursor.revision) {
    throw new RuntimeResyncRequired();
  }
  return next;
}
