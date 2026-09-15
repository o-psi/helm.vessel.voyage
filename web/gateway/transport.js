import http from 'node:http';
import https from 'node:https';
import { lookup } from 'node:dns/promises';
import { BlockList, isIP } from 'node:net';
import { WebSocket } from 'ws';
import { exact, UUID } from './protocol.js';

const excluded = new BlockList();
for (const [address, prefix] of [ ['0.0.0.0',8], ['10.0.0.0',8], ['100.64.0.0',10], ['127.0.0.0',8],
  ['169.254.0.0',16], ['172.16.0.0',12], ['192.0.0.0',24], ['192.0.2.0',24], ['192.88.99.0',24],
  ['192.168.0.0',16], ['198.18.0.0',15], ['198.51.100.0',24], ['203.0.113.0',24], ['224.0.0.0',4], ['240.0.0.0',4] ]) excluded.addSubnet(address, prefix, 'ipv4');
const global6 = new BlockList(); global6.addSubnet('2000::', 3, 'ipv6');
for (const [address, prefix] of [['2001::',23], ['2001:db8::',32], ['2002::',16], ['3fff::',20]]) excluded.addSubnet(address, prefix, 'ipv6');
export function isPublicAddress(address) {
  const family = isIP(address);
  if (family === 4) return !excluded.check(address, 'ipv4');
  return family === 6 && !address.includes('%') && global6.check(address, 'ipv6') && !excluded.check(address, 'ipv6');
}
export function endpoint(value, pairing = false) {
  if (typeof value !== 'string') throw Error('invalid endpoint');
  const u = new URL(value);
  if (u.protocol !== (pairing ? 'https:' : 'wss:') || u.port || u.username || u.password || u.search || u.hash || (u.href !== value && !(pairing && u.origin === value)) ||
      (pairing ? u.pathname !== '/' : u.pathname !== '/v1/vessel/socket')) throw Error('invalid endpoint');
  return u;
}
export function validateConnection(v) {
  if (!exact(v, ['url', 'token', 'grant_id', 'vessel_id']) || !UUID.test(v.grant_id) || !UUID.test(v.vessel_id) ||
    typeof v.token !== 'string' || v.token.length > 4096 || !/^[\x21-\x7e]+$/.test(v.token)) throw Error('invalid connection');
  endpoint(v.url); return v;
}
// Resolve every returned address and pin one on the actual socket. No second DNS
// lookup, pooled socket, redirect, proxy, or browser-selected authorization URL.
export async function publicTarget(value, pairing = false, resolve = lookup) {
  const url = endpoint(value, pairing), host = url.hostname.replace(/^\[|\]$/g, '');
  let timer;
  const resolution = () => Promise.race([resolve(host, { all: true, verbatim: true }), new Promise((_, reject) => { timer = setTimeout(() => reject(Error('DNS deadline')), 5000); })]).finally(() => clearTimeout(timer));
  const addresses = isIP(host) ? [{ address: host, family: isIP(host) }] : await resolution();
  if (!addresses.length || addresses.some(a => !isPublicAddress(a.address) || isIP(a.address) !== a.family)) throw Error('non-public endpoint');
  const { address, family } = addresses[0];
  const pinnedLookup = (_host, options, cb) => {
    if (typeof options === 'function') { cb = options; options = {}; }
    if (options?.all) cb(null, [{ address, family }]); else cb(null, address, family);
  };
  return { url, options: { lookup: pinnedLookup, family, autoSelectFamily: false, agent: false, rejectUnauthorized: true,
    ...(isIP(host) ? {} : { servername: host }) } };
}
export function jsonPost(url, body, { headers = {}, timeout = 5000, maxBytes = 65536, ...options } = {}) {
  return new Promise((resolve, reject) => {
    const bytes = Buffer.from(JSON.stringify(body));
    const request = (url.protocol === 'https:' ? https : http).request(url, { ...options, method: 'POST',
      headers: { ...headers, 'content-type': 'application/json', 'content-length': bytes.length } });
    const timer = setTimeout(() => request.destroy(Error('request deadline')), timeout);
    let response;
    const fail = () => { response?.destroy(); request.destroy(); clearTimeout(timer); reject(Error('service unavailable')); };
    request.on('error', fail);
    request.on('response', res => {
      response = res;
      if (res.statusCode !== 200 || !/^application\/json(?:\s*;|$)/i.test(res.headers['content-type'] ?? '')) { fail(); return; }
      const chunks = []; let size = 0;
      res.on('error', fail); res.on('aborted', fail);
      res.on('data', chunk => { size += chunk.length; if (size > maxBytes) fail(); else chunks.push(chunk); });
      res.on('end', () => { clearTimeout(timer); try { resolve(JSON.parse(Buffer.concat(chunks).toString('utf8'))); } catch { fail(); } });
    });
    request.end(bytes);
  });
}
export function openSocket(target, v, limits) {
  return new WebSocket(target.url, 'voyage.vessel.v1', { ...target.options, maxPayload: limits.bytes, perMessageDeflate: false,
    handshakeTimeout: limits.helloMs, followRedirects: false,
    headers: { authorization: `Bearer ${v.token}`, 'x-voyage-grant': v.grant_id, 'x-voyage-vessel': v.vessel_id } });
}
export const transport = Object.freeze({ publicTarget, jsonPost, openSocket });
