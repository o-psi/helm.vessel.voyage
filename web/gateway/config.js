export function validateConfig(c) {
  if (typeof c.secret !== 'string' || Buffer.byteLength(c.secret) < 32 || !/^[\x21-\x7e]+$/.test(c.secret)) throw Error('invalid gateway secret');
  const origin = new URL(c.origin);
  if (!(origin.protocol === 'https:' || (origin.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(origin.hostname))) || origin.origin !== c.origin) throw Error('invalid web origin');
  const auth = new URL(c.authUrl);
  if (auth.protocol !== 'http:' || !['127.0.0.1', '[::1]'].includes(auth.hostname) || auth.username || auth.password || auth.search || auth.hash || auth.href !== c.authUrl) throw Error('invalid authorization endpoint');
  if (c.host && !['127.0.0.1', '::1'].includes(c.host)) throw Error('listener must be loopback');
  if ('allowLoopback' in c || 'vessels' in c) throw Error('obsolete gateway configuration');
  return c;
}
export async function loadConfig(env = process.env) {
  if (env.HELM_WEB_ALLOW_LOOPBACK_WS || env.HELM_WEB_VESSELS_FILE) throw Error('obsolete gateway configuration');
  const port = Number(env.HELM_WEB_GATEWAY_PORT ?? 8787);
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw Error('invalid listen port');
  return validateConfig({ secret: env.HELM_WEB_GATEWAY_SECRET, origin: env.HELM_WEB_ORIGIN,
    authUrl: env.HELM_WEB_AUTH_URL, host: env.HELM_WEB_GATEWAY_HOST ?? '127.0.0.1', port });
}
