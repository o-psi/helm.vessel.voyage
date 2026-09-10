import net from 'node:net';
import dns from 'node:dns/promises';
import http from 'node:http';
import crypto from 'node:crypto';

export class Refusal extends Error {
  constructor(code) { super(code); this.code = code; }
}
export const refuse = code => { throw new Refusal(code); };
export const token = () => crypto.randomBytes(32).toString('base64url');
export function digest(value) {
  const canonical = v => Array.isArray(v) ? v.map(canonical) : v && typeof v === 'object'
    ? Object.fromEntries(Object.keys(v).sort().map(k => [k, canonical(v[k])])) : v;
  return crypto.createHash('sha256').update(JSON.stringify(canonical(value))).digest('hex');
}
export function origin(value) {
  let u; try { u = new URL(value); } catch { refuse('invalid_url'); }
  if (!['http:', 'https:'].includes(u.protocol) || u.username || u.password) refuse('invalid_url');
  return u.origin;
}
export function publicAddress(ip) {
  if (net.isIP(ip) === 6) {
    // Only global unicast; mapped IPv4, local, transition, documentation ranges denied.
    const s = ip.toLowerCase();
    return /^[23][0-9a-f]{3}:/.test(s) && !s.startsWith('2001:') && !s.startsWith('2002:');
  }
  if (net.isIP(ip) !== 4) return false;
  const [a,b,c] = ip.split('.').map(Number);
  return !(a === 0 || a === 10 || a === 127 || a >= 224 || (a === 169 && b === 254)
    || (a === 172 && b >= 16 && b <= 31) || (a === 192 && (b === 168 || b === 0 || (b === 88 && c === 99)))
    || (a === 100 && b >= 64 && b <= 127) || (a === 198 && (b === 18 || b === 19 || (b === 51 && c === 100)))
    || (a === 203 && b === 0 && c === 113));
}

// Chromium's only network path: resolve, classify, then connect to that exact IP.
// Browser routing additionally checks every URL (including redirects and frames).
export async function networkProxy(allowed) {
  const username = 'helm', password = token();
  const auth = `Basic ${Buffer.from(`${username}:${password}`).toString('base64')}`;
  const sockets = new Set();
  const check = async raw => {
    const o = origin(raw), grant = allowed.get(o);
    if (!grant) refuse('origin_not_allowed');
    const u = new URL(raw), host = u.hostname.replace(/^\[|\]$/g, '');
    const addresses = net.isIP(host) ? [{address:host}] : await dns.lookup(host, {all:true});
    if (!addresses.length || (!grant.private_network && addresses.some(a => !publicAddress(a.address)))) refuse('private_network_denied');
    return {u, address:addresses[0].address};
  };
  const server = http.createServer(async (req,res) => {
    if (req.headers['proxy-authorization'] !== auth) { res.writeHead(407, {'Proxy-Authenticate':'Basic realm="Helm"'}).end(); return; }
    try {
      const {u,address} = await check(req.url);
      if (u.protocol !== 'http:') refuse('invalid_url');
      const headers = {...req.headers,host:u.host};
      delete headers['proxy-authorization']; delete headers['proxy-connection'];
      const upstream = http.request({host:address,port:u.port || 80,method:req.method,path:u.pathname+u.search,headers,timeout:15000}, reply => {
        res.writeHead(reply.statusCode,reply.headers); reply.pipe(res);
      });
      upstream.on('error',() => res.destroy()); upstream.on('timeout',() => upstream.destroy());
      req.on('aborted',() => upstream.destroy()); req.pipe(upstream);
    } catch { res.writeHead(403).end(); }
  });
  server.on('connect',async (req,client,head) => {
    if (req.headers['proxy-authorization'] !== auth) { client.end('HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm="Helm"\r\n\r\n'); return; }
    try {
      const {u,address} = await check(`https://${req.url}`);
      const remote = net.connect({host:address,port:Number(u.port || 443)});
      sockets.add(remote); remote.once('close',() => sockets.delete(remote));
      remote.setTimeout(30000,() => remote.destroy());
      remote.once('connect',() => { client.write('HTTP/1.1 200 Connection Established\r\n\r\n'); if(head.length)remote.write(head); client.pipe(remote); remote.pipe(client); });
      remote.on('error',() => client.destroy()); client.on('error',() => remote.destroy()); client.on('close',() => remote.destroy());
    } catch { client.end('HTTP/1.1 403 Forbidden\r\n\r\n'); }
  });
  server.on('connection',socket => { sockets.add(socket); socket.on('close',() => sockets.delete(socket)); });
  server.on('clientError',(_e,s) => s.destroy());
  await new Promise(resolve => server.listen(0,'127.0.0.1',resolve));
  return {server:`http://127.0.0.1:${server.address().port}`,username,password,
    fence:() => {for(const s of sockets)s.destroy();},
    close:async () => { for (const s of sockets) s.destroy(); await new Promise(resolve => server.close(resolve)); }};
}
