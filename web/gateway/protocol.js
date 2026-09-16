// Mirrors the deliberately restricted subset of voyage-protocol::{duplex,vessel}.
export const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
export const object = v => v !== null && typeof v === 'object' && !Array.isArray(v);
export const exact = (v, required, optional = []) => object(v) && required.every(k => Object.hasOwn(v, k)) && Object.keys(v).every(k => [...required, ...optional].includes(k));
const uuid = v => typeof v === 'string' && UUID.test(v);
const uint = v => Number.isSafeInteger(v) && v >= 0;
const text = v => typeof v === 'string' && v.length > 0 && v.length <= 4096 && !v.includes('\0');
const nullableText = v => v === null || text(v);
const account = v => exact(v, ['account_id','connection_id','identity_generation','connection_revision','transport']) && uuid(v.account_id) && uuid(v.connection_id) && uint(v.identity_generation) && uint(v.connection_revision) && ['openai_responses','openai_chat','chatgpt_oauth','anthropic'].includes(v.transport);
function accountCommand(c) {
  if (c.op === 'set_access') return exact(c,['op','session_id','incarnation','command_id','expected_revision','expires_at_ms','access']) && uuid(c.session_id) && uuid(c.incarnation) && uuid(c.command_id) && uint(c.expected_revision) && uint(c.expires_at_ms) && ['read-only','approval','unrestricted'].includes(c.access);
  if (c.op === 'accounts') return exact(c,['op','workspace','transport']) && text(c.workspace) && c.transport === null;
  if (c.op === 'account_defaults') return exact(c,['op','workspace']) && text(c.workspace);
  if (c.op === 'account_usage') return exact(c,['op','workspace','account','refresh']) && text(c.workspace) && account(c.account) && typeof c.refresh === 'boolean';
  if (c.op === 'account_models') return exact(c,['op','workspace','account']) && text(c.workspace) && account(c.account);
  const inference = ['account','model','reasoning_effort','service_tier'];
  if (['start_account','resolve_start_account'].includes(c.op)) return exact(c,['op','workspace','command_id','session_id',...inference]) && text(c.workspace) && uuid(c.command_id) && uuid(c.session_id) && account(c.account) && text(c.model) && nullableText(c.reasoning_effort) && nullableText(c.service_tier);
  if (c.op === 'set_account_inference') return exact(c,['op','session_id','incarnation','command_id','expected_revision','expires_at_ms',...inference]) && uuid(c.session_id) && uuid(c.incarnation) && uuid(c.command_id) && uint(c.expected_revision) && uint(c.expires_at_ms) && account(c.account) && text(c.model) && nullableText(c.reasoning_effort) && nullableText(c.service_tier);
  return false;
}
const fields = {
  capabilities: [], catalogue: [], inspect: ['session_id'], snapshot: [], decisions: [],
  history: ['offset', 'limit'], message_chunk: ['index', 'offset', 'limit', 'expected_revision'],
  run_output: ['run_id', 'offset', 'limit'], receipt: ['command_id'],
  events: ['after', 'limit', 'wait_ms'],
  submit: ['command_id', 'expected_revision', 'expires_at_ms', 'prompt'],
  steer: ['command_id', 'expected_revision', 'expires_at_ms', 'run_id', 'prompt'],
  cancel: ['command_id', 'expected_revision', 'expires_at_ms', 'run_id'],
  respond: ['command_id', 'expected_revision', 'expires_at_ms', 'run_id', 'decision_id', 'response'],
};
export function validCommand(f) {
  if (!exact(f, ['type', 'request_id', 'request']) || f.type !== 'command' || !uuid(f.request_id) || !exact(f.request, ['protocol', 'command']) || f.request.protocol !== 1) return false;
  const c = f.request.command;
  if (object(c) && accountCommand(c)) return true;
  if (!object(c) || !Object.hasOwn(fields, c.op)) return false;
  const vessel = ['capabilities', 'catalogue', 'inspect'].includes(c.op);
  const live = ['steer', 'cancel', 'respond'].includes(c.op);
  const required = ['op', ...fields[c.op], ...(!vessel ? ['session_id'] : []), ...(live ? ['incarnation'] : [])];
  const optional = [...(!vessel && !live ? ['incarnation'] : []), ...(c.op === 'history' ? ['expected_revision'] : [])];
  if (!exact(c, required, optional)) return false;
  return Object.entries(c).every(([k,v]) => {
    if (k === 'op' || k === 'response') return true; // response is serde_json::Value; owner validates decision kind.
    if (k === 'incarnation' && !live && v === null) return true;
    if (k.endsWith('_id') || k === 'incarnation') return uuid(v);
    if (k === 'prompt') return typeof v === 'string' && v.length > 0;
    if (k === 'expected_revision' && c.op === 'history' && v === null) return true;
    if (!uint(v)) return false;
    if (k === 'limit') return v >= 1 && v <= (['message_chunk', 'run_output'].includes(c.op) ? 65536 : 128);
    if (k === 'wait_ms') return v <= 10000;
    return true;
  });
}
export function validHello(f, vesselId) {
  return exact(f, ['type', 'protocol', 'socket_id', 'vessel_id']) && f.type === 'hello' && f.protocol === 1 && uuid(f.socket_id) && f.vessel_id === vesselId;
}
export function validReply(f) {
  return exact(f, ['type', 'request_id', 'response']) && f.type === 'reply' && uuid(f.request_id) &&
    exact(f.response, ['protocol', 'result', 'error', 'outcome_unknown']) && f.response.protocol === 1 &&
    (f.response.error === null || typeof f.response.error === 'string') && typeof f.response.outcome_unknown === 'boolean';
}

// Rust capabilities are a JSON value, not a command catalogue. Project a safe
// view rather than forwarding unknown future execution/credential metadata.
export function restrictCapabilities(value, vesselId) {
  if (!object(value) || value.protocol !== 1 || value.vessel_id !== vesselId || !Array.isArray(value.features) || !value.features.every(v => typeof v === 'string')) throw Error('invalid capabilities');
  const features = new Set(['sqlite_catalogue', 'catalogue', 'scoped_catalogue', 'inspect', 'durable_receipts', 'history_paging', 'events', 'duplex_socket', 'decisions', 'grant_revocation', 'revocation', 'provider_accounts', 'account_start', 'start_resolution']);
  const result = { protocol: 1, vessel_id: vesselId, features: value.features.filter(f => features.has(f)) };
  if (typeof value.version === 'string') result.version = value.version;
  if (typeof value.scope === 'string') result.scope = value.scope;
  if (uint(value.expires_at_ms)) result.expires_at_ms = value.expires_at_ms;
  if (uint(value.grant_revision)) result.grant_revision = value.grant_revision;
  if (uuid(value.session_id)) result.session_id = value.session_id;
  result.rights = Array.isArray(value.rights) ? value.rights.filter(v => typeof v === 'string') : [];
  result.workspaces = Array.isArray(value.workspaces) ? value.workspaces.filter(v => object(v) && text(v.path)).map(v => ({path:v.path, name:typeof v.name === 'string' ? v.name : v.path})) : [];
  return result;
}
