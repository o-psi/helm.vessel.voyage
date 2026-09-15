import { open } from 'node:fs/promises';
import { constants } from 'node:fs';
import { isAbsolute } from 'node:path';
import { exact, UUID } from './protocol.js';
export function validateConfig(c) {
  if (typeof c.secret !== 'string' || Buffer.byteLength(c.secret) < 32) throw Error('invalid gateway secret');
  const origin = new URL(c.origin);
  if (!(origin.protocol === 'https:' || (origin.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(origin.hostname))) || origin.origin !== c.origin) throw Error('invalid web origin');
  if (!c.vessels || typeof c.vessels !== 'object' || Array.isArray(c.vessels) || !Object.keys(c.vessels).length) throw Error('invalid vessel configuration');
  for (const [alias, v] of Object.entries(c.vessels)) {
    if (!/^[a-zA-Z0-9_-]{1,64}$/.test(alias) || !exact(v, ['url', 'token', 'grant_id', 'vessel_id']) ||
      !UUID.test(v.grant_id) || !UUID.test(v.vessel_id) || typeof v.token !== 'string' || !/^[\x21-\x7e]+$/.test(v.token)) throw Error('invalid vessel configuration');
    const u = new URL(v.url);
    if (u.username || u.password || u.search || u.hash || u.pathname !== '/v1/vessel/socket' ||
      !(u.protocol === 'wss:' || (c.allowLoopback && u.protocol === 'ws:' && ['127.0.0.1', '[::1]'].includes(u.hostname)))) throw Error('invalid vessel endpoint');
  }
  return c;
}
export async function loadConfig(env = process.env) {
  // Refuse symlinks and group/world-readable credential files; no content in errors.
  if (typeof env.HELM_WEB_VESSELS_FILE !== 'string' || !isAbsolute(env.HELM_WEB_VESSELS_FILE)) throw Error('private file path must be absolute');
  const file = await open(env.HELM_WEB_VESSELS_FILE, constants.O_RDONLY | constants.O_NOFOLLOW);
  let vessels;
  try {
    const stat = await file.stat();
    if (!stat.isFile() || (stat.mode & 0o077) || (process.getuid && stat.uid !== process.getuid()) || stat.size > 65536) throw Error('vessel file must be private and owned by gateway user');
    vessels = JSON.parse(await file.readFile('utf8'));
  } finally { await file.close(); }
  const port = Number(env.HELM_WEB_GATEWAY_PORT ?? 8787);
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw Error('invalid listen port');
  return validateConfig({ secret: env.HELM_WEB_GATEWAY_SECRET, origin: env.HELM_WEB_ORIGIN, vessels,
    admissionNotBefore: Date.now() + 60000, allowLoopback: env.HELM_WEB_ALLOW_LOOPBACK_WS === '1', host: env.HELM_WEB_GATEWAY_HOST ?? '127.0.0.1', port });
}
