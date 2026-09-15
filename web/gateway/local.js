import { timingSafeEqual, randomUUID } from 'node:crypto';
import { exact, UUID, validHello, validReply, restrictCapabilities } from './protocol.js';
import { validateConnection } from './transport.js';

export async function probe(connection, io, limits) {
  const v = validateConnection(connection);
  const target = await io.publicTarget(v.url);
  return new Promise((resolve, reject) => {
    const ws = io.openSocket(target, v, limits), request_id = randomUUID();
    let hello = false, finished = false;
    const finish = (err, value) => { if (finished) return; finished = true; clearTimeout(timer); ws.terminate(); err ? reject(Error('probe refused')) : resolve(value); };
    const timer = setTimeout(() => finish(true), limits.helloMs + limits.requestMs);
    ws.on('error', () => finish(true)); ws.on('close', () => finish(true));
    ws.on('message', (data, binary) => {
      try {
        if (binary) throw Error();
        const frame = JSON.parse(data.toString());
        if (!hello) {
          if (ws.protocol !== 'voyage.vessel.v1' || !validHello(frame, v.vessel_id)) throw Error();
          hello = true;
          ws.send(JSON.stringify({ type: 'command', request_id, request: { protocol: 1, command: { op: 'capabilities' } } }));
        } else {
          if (!validReply(frame) || frame.request_id !== request_id || frame.response.error !== null || frame.response.outcome_unknown !== false) throw Error();
          finish(false, restrictCapabilities(frame.response.result, v.vessel_id));
        }
      } catch { finish(true); }
    });
  });
}
export function localHandler(config, io, limits) {
  let active = 0;
  return async (req, res) => {
    res.setHeader('cache-control', 'no-store');
    const reply = (status, value = { error: 'gateway refused' }) => {
      let body = JSON.stringify(value);
      if (Buffer.byteLength(body) > 65536) { status = 502; body = '{"error":"gateway refused"}'; }
      if (!res.destroyed) { res.writeHead(status, { 'content-type': 'application/json' }); res.end(body); }
    };
    if (!['/pair', '/probe'].includes(req.url) || req.method !== 'POST') { reply(404); return; }
    const expected = Buffer.from(`Bearer ${config.secret}`), supplied = Buffer.from(req.headers.authorization ?? '');
    if (!['127.0.0.1', '::1', '::ffff:127.0.0.1'].includes(req.socket.remoteAddress) || req.headers.origin ||
      req.rawHeaders.filter((v,i) => i % 2 === 0 && v.toLowerCase() === 'authorization').length !== 1 ||
      supplied.length !== expected.length || !timingSafeEqual(supplied, expected)) { reply(403); return; }
    if (active >= 8) { reply(503); return; }
    active++;
    const timer = setTimeout(() => req.destroy(), limits.requestMs);
    try {
      if (!/^application\/json(?:\s*;|$)/i.test(req.headers['content-type'] ?? '')) throw Error();
      let size = 0; const chunks = [];
      for await (const chunk of req) { size += chunk.length; if (size > 16384) throw Error(); chunks.push(chunk); }
      const value = JSON.parse(Buffer.concat(chunks).toString('utf8'));
      if (req.url === '/probe') {
        if (!exact(value, ['connection'])) throw Error();
        reply(200, await probe(value.connection, io, limits));
      } else {
        if (!exact(value, ['endpoint', 'principal_id', 'invitation_id', 'command_id', 'code', 'vessel_id']) ||
          !['principal_id', 'invitation_id', 'command_id', 'vessel_id'].every(k => typeof value[k] === 'string' && UUID.test(value[k])) ||
          typeof value.code !== 'string' || !/^[\x21-\x7e]{1,4096}$/.test(value.code)) throw Error();
        const target = await io.publicTarget(value.endpoint, true);
        const url = new URL('/v1/vessel/pair', target.url);
        const { principal_id, invitation_id, command_id, code } = value;
        const result = await io.jsonPost(url, { protocol: 1, principal_id, invitation_id, command_id, code }, {
          ...target.options, headers: { 'x-voyage-vessel': value.vessel_id }, timeout: limits.requestMs });
        if (!exact(result, ['protocol', 'result', 'error', 'outcome_unknown']) || result.protocol !== 1 || result.error !== null || result.outcome_unknown !== false || !result.result) throw Error();
        reply(200, result);
      }
    } catch { reply(502); } finally { clearTimeout(timer); active--; }
  };
}
