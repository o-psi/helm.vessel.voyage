// Offline source-function fixture only; never starts Chromium or product binaries.
// Usage: node helm/browser/ux272-offline-boundaries.mjs /path/to/helper.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import vm from 'node:vm';
const source = await fs.readFile(process.argv[2], 'utf8');
function section(start, end) {
  assert.equal(source.split(start).length, 2);
  const a = source.indexOf(start), b = source.indexOf(end, a);
  assert.ok(b > a);
  return source.slice(a, b);
}
const S = {epoch: 7, receipts: new Map(), queue: [], binding: null};
let authorityCalls = 0;
const sandbox = {
  S, ID: /^[a-z]+$/, limits: {max_receipts: 2},
  digest: req => JSON.stringify(req),
  refuse: code => { throw new Error(code); },
  authority: epoch => { authorityCalls++; if (epoch !== S.epoch) throw new Error('authority_fenced'); },
  normalizeAction: () => {}, durable: async () => { throw new Error('unexpected admission'); },
  drain: () => { throw new Error('unexpected dispatch'); },
};
vm.createContext(sandbox);
vm.runInContext(section('function localReason(', '\nasync function dispatch(')
  + section('async function action(', '\nfunction safeName('), sandbox);
const req = {id: 'stable', epoch: 7, capture_epoch: 7, action_sha256: 'a'.repeat(64), action: {kind: 'observe'}};
for (const state of ['queued', 'dispatched', 'unknown', 'completed', 'cancelled_before_dispatch', 'refused']) {
  S.receipts.set(req.id, {digest: JSON.stringify(req), state, private_canary: 'synthetic-not-for-wire'});
  const before = authorityCalls;
  const result = await sandbox.action(req);
  assert.equal(authorityCalls, before); // Receipt lookup never dispatches again.
  assert.equal(result.result.request_id, req.id);
  assert.equal(result.result.action_sha256, req.action_sha256);
  assert.equal(result.result.image, null);
  assert.equal(result.result.file, null);
  assert.equal(result.result.page_id, null);
  assert.equal(result.result.observation_id, null);
  assert.equal(result.result.text, 'duplicate_content_withheld');
  assert.ok(!JSON.stringify(result).includes('synthetic-not-for-wire'));
  assert.equal(result.result.state, state === 'completed' ? 'completed' :
    ['dispatched', 'unknown'].includes(state) ? 'unresolved' :
    state === 'cancelled_before_dispatch' ? 'cancelled' : 'refused');
  await assert.rejects(sandbox.action({...req, action_sha256: 'b'.repeat(64)}), /request_id_conflict/);
}
S.receipts.clear();
await assert.rejects(sandbox.action({...req, epoch: 6}), /authority_fenced/);
await assert.rejects(sandbox.action({...req, capture_epoch: 6}), /capture_fenced/);
S.binding = {};
await assert.rejects(sandbox.action(req), /binding_required/);
S.binding = null;
await assert.rejects(sandbox.action({...req, action_sha256: 'bad'}), /invalid_action_digest/);
S.queue = Array(16).fill({});
await assert.rejects(sandbox.action(req), /resource_limit/);
assert.equal(S.receipts.size, 0);
console.log('PASS: 6 duplicate receipt states with withheld content; 6 conflicting identities; 5 pre-admission refusals; no dispatch');
