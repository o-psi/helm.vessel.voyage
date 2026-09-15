import http from 'node:http';
import { createHmac, timingSafeEqual } from 'node:crypto';
import { WebSocket, WebSocketServer } from 'ws';
import { exact, UUID, validCommand, validHello, validReply, restrictCapabilities } from './protocol.js';
import { validateConfig } from './config.js';

export const LIMITS = Object.freeze({ bytes: 4 * 1024 * 1024, connections: 64, perSubject: 4, inflight: 32,
  rate: 60, authMs: 5000, helloMs: 5000, requestMs: 15000, heartbeatMs: 5000, seen: 10000 });

export function verifyTicket(ticket, secret, now = Date.now()) {
  if (typeof ticket !== 'string' || ticket.length > 4096 || !/^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(ticket)) throw Error('authentication refused');
  const [payload, signature] = ticket.split('.');
  const sig = Buffer.from(signature, 'base64url');
  const expected = createHmac('sha256', secret).update(payload).digest();
  if (sig.toString('base64url') !== signature || sig.length !== expected.length || !timingSafeEqual(sig, expected)) throw Error('authentication refused');
  const bytes = Buffer.from(payload, 'base64url');
  if (bytes.toString('base64url') !== payload) throw Error('authentication refused');
  const c = JSON.parse(bytes.toString('utf8'));
  if (!exact(c, ['aud', 'sub', 'jti', 'exp', 'vessel']) || c.aud !== 'helm-web-gateway' ||
    typeof c.sub !== 'string' || !/^[a-f0-9]{64}$/i.test(c.sub) || typeof c.jti !== 'string' || !UUID.test(c.jti) ||
    !Number.isSafeInteger(c.exp) || c.exp <= now / 1000 || c.exp > Math.floor(now / 1000) + 60 ||
    typeof c.vessel !== 'string') throw Error('authentication refused');
  return c;
}

export function createGateway(config, limits = LIMITS) {
  validateConfig(config);
  const tickets = new Map(), subjects = new Map(), connections = new Set();
  const server = http.createServer((_req, res) => { res.writeHead(404); res.end(); });
  server.headersTimeout = 5000;
  server.requestTimeout = 5000;
  server.maxConnections = limits.connections + 16;
  const wss = new WebSocketServer({ noServer: true, maxPayload: limits.bytes, perMessageDeflate: false, handleProtocols: () => false });
  server.on('upgrade', (req, socket, head) => {
    const origins = req.rawHeaders.filter((v, i) => i % 2 === 0 && v.toLowerCase() === 'origin');
    if (Date.now() < (config.admissionNotBefore ?? 0) || req.url !== '/socket' || origins.length !== 1 || req.headers.origin !== config.origin || connections.size >= limits.connections) {
      socket.end('HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n'); return;
    }
    wss.handleUpgrade(req, socket, head, ws => wss.emit('connection', ws));
  });
  wss.on('connection', browser => {
    connections.add(browser);
    let upstream, claims, subjectKey, leaseTimer, helloTimer, heartbeat, ready = false, stopped = false;
    let budget = limits.rate, budgetAt = Date.now();
    const pending = new Map(), seen = new Set();
    const authTimer = setTimeout(() => stop(1008, 'authentication required'), limits.authMs);
    function stop(code = 1008, reason = 'gateway refused') {
      if (stopped) return;
      stopped = true;
      clearTimeout(authTimer); clearTimeout(leaseTimer); clearTimeout(helloTimer); clearInterval(heartbeat);
      for (const entry of pending.values()) clearTimeout(entry.timer);
      pending.clear(); connections.delete(browser);
      if (subjectKey) { const n = subjects.get(subjectKey) - 1; if (n) subjects.set(subjectKey, n); else subjects.delete(subjectKey); }
      for (const ws of [browser, upstream]) if (ws) {
        if (ws.readyState === WebSocket.OPEN) ws.close(code, reason); else if (ws.readyState !== WebSocket.CLOSED) ws.terminate();
        const kill = setTimeout(() => ws.terminate(), 1000); kill.unref(); ws.once('close', () => clearTimeout(kill));
      }
    }
    function send(ws, data) {
      const text = typeof data === 'string' ? data : JSON.stringify(data);
      if (ws.readyState !== WebSocket.OPEN || ws.bufferedAmount + Buffer.byteLength(text) > limits.bytes) { stop(1013, 'gateway capacity'); return; }
      ws.send(text, err => { if (err) stop(1011, 'transport closed'); });
    }
    function authenticate(frame) {
      if (!exact(frame, ['type', 'ticket'])) throw Error();
      const next = verifyTicket(frame.ticket, config.secret);
      if (!Object.hasOwn(config.vessels, next.vessel) || (claims && (next.sub !== claims.sub || next.vessel !== claims.vessel))) throw Error();
      const now = Date.now();
      for (const [id, expiry] of tickets) if (expiry <= now) tickets.delete(id);
      if (tickets.has(next.jti) || tickets.size >= limits.seen) throw Error();
      if (!claims) {
        subjectKey = next.sub;
        if ((subjects.get(subjectKey) ?? 0) >= limits.perSubject) { subjectKey = undefined; throw Error(); }
        subjects.set(subjectKey, (subjects.get(subjectKey) ?? 0) + 1);
      }
      tickets.set(next.jti, next.exp * 1000); claims = next;
      clearTimeout(authTimer); clearTimeout(leaseTimer);
      leaseTimer = setTimeout(() => stop(1008, 'lease expired'), next.exp * 1000 - now);
      if (upstream) { if (ready) send(browser, { type: 'ready', vessel_id: config.vessels[claims.vessel].vessel_id }); return; }
      const v = config.vessels[next.vessel];
      upstream = new WebSocket(v.url, 'voyage.vessel.v1', { maxPayload: limits.bytes, perMessageDeflate: false,
        handshakeTimeout: limits.helloMs, followRedirects: false,
        headers: { authorization: `Bearer ${v.token}`, 'x-voyage-grant': v.grant_id, 'x-voyage-vessel': v.vessel_id } });
      helloTimer = setTimeout(() => stop(1011, 'upstream unavailable'), limits.helloMs);
      upstream.on('error', () => stop(1011, 'upstream unavailable'));
      upstream.on('close', () => stop(1011, 'upstream closed'));
      upstream.on('message', (data, binary) => {
        try {
          if (binary || Date.now() >= claims.exp * 1000) throw Error();
          const frame = JSON.parse(data.toString());
          if (!ready) {
            if (upstream.protocol !== 'voyage.vessel.v1' || !validHello(frame, v.vessel_id)) throw Error();
            clearTimeout(helloTimer); ready = true; send(browser, { type: 'ready', vessel_id: config.vessels[claims.vessel].vessel_id }); return;
          }
          if (!validReply(frame) || !pending.has(frame.request_id)) throw Error();
          const entry = pending.get(frame.request_id);
          clearTimeout(entry.timer); pending.delete(frame.request_id);
          if (entry.op === 'capabilities' && frame.response.error === null) {
            frame.response.result = restrictCapabilities(frame.response.result, v.vessel_id);
            send(browser, frame);
          } else send(browser, data.toString());
        } catch { stop(1008, 'invalid upstream frame'); }
      });
      for (const ws of [browser, upstream]) { ws.alive = true; ws.on('pong', () => { ws.alive = true; }); }
      heartbeat = setInterval(() => {
        for (const ws of [browser, upstream]) if (ws.readyState === WebSocket.OPEN) {
          if (!ws.alive) { stop(1011, 'heartbeat timeout'); return; }
          ws.alive = false; ws.ping();
        }
      }, limits.heartbeatMs);
    }
    browser.on('error', () => stop(1011, 'transport closed'));
    browser.on('close', () => stop(1000, 'transport closed'));
    browser.on('message', (data, binary) => {
      try {
        if (stopped || binary || (claims && Date.now() >= claims.exp * 1000)) throw Error();
        const now = Date.now(); budget = Math.min(limits.rate, budget + (now - budgetAt) * limits.rate / 1000); budgetAt = now;
        if (budget < 1) { stop(1008, 'rate exceeded'); return; } budget--;
        const frame = JSON.parse(data.toString());
        if (frame.type === 'authenticate') { authenticate(frame); return; }
        if (!ready || !validCommand(frame) || seen.has(frame.request_id)) throw Error();
        if (pending.size >= limits.inflight || seen.size >= limits.seen) { stop(1013, 'gateway capacity'); return; }
        seen.add(frame.request_id);
        pending.set(frame.request_id, { op: frame.request.command.op, timer: setTimeout(() => stop(1011, 'request outcome unknown'), limits.requestMs) });
        send(upstream, data.toString());
      } catch { stop(1008, 'gateway refused'); }
    });
  });
  return { server, close: async () => {
    for (const ws of connections) ws.terminate();
    await new Promise(resolve => wss.close(resolve));
    await new Promise(resolve => server.close(resolve));
  } };
}
