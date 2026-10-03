import test from 'node:test';
import assert from 'node:assert/strict';
import net from 'node:net';
import {once} from 'node:events';
import {networkProxy} from '../security.mjs';

const bounded=async promise=>{
 let timer;try{return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('owned tunnel did not retire')),1500);})]);}
 finally{clearTimeout(timer);}
};
async function fixture(t,accept){
 const peers=new Set(),clients=new Set();const server=net.createServer(socket=>{peers.add(socket);socket.on('error',()=>{});socket.once('close',()=>peers.delete(socket));accept(socket);});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 const authority=`127.0.0.1:${server.address().port}`;
 const proxy=await networkProxy(new Map([[`https://${authority}`,{private_network:true}]]));
 t.after(async()=>{for(const socket of clients)socket.destroy();for(const socket of peers)socket.destroy();await proxy.close();await new Promise(resolve=>server.close(resolve));});
 async function connect(){
  const url=new URL(proxy.server),client=net.connect({host:url.hostname,port:url.port});clients.add(client);client.on('error',()=>{});client.once('close',()=>clients.delete(client));
  await once(client,'connect');
  const response=new Promise((resolve,reject)=>{
   let bytes='';const data=chunk=>{bytes+=chunk.toString('ascii');if(bytes.includes('\r\n\r\n')){client.off('data',data);assert.match(bytes,/^HTTP\/1\.1 200 Connection Established\r\n/);resolve();}};
   client.on('data',data);client.once('error',reject);
  });
  client.write(`CONNECT ${authority} HTTP/1.1\r\nHost: ${authority}\r\nProxy-Authorization: Basic ${Buffer.from(`${proxy.username}:${proxy.password}`).toString('base64')}\r\n\r\n`);
  await bounded(response);return client;
 }
 return {connect,peers};
}

test('the unchanged 30s CONNECT idle timeout retires Chromium side and a fresh tunnel still works',{timeout:5000},async t=>{
 const original=net.Socket.prototype.setTimeout;let deadlines=0;
 // Only the owned proxy's declared 30s timeout is accelerated; production has
 // no test override or widened timeout/authority setting.
 t.mock.method(net.Socket.prototype,'setTimeout',function(ms,...args){if(ms===30000){deadlines++;return original.call(this,80,...args);}return original.call(this,ms,...args);});
 const f=await fixture(t,socket=>socket.pipe(socket));
 const first=await f.connect();const firstClosed=new Promise(resolve=>first.once('close',resolve));
 const echoed=once(first,'data');first.write('first');assert.equal((await echoed)[0].toString(),'first');
 await bounded(firstClosed);assert.equal(first.destroyed,true);
 const second=await f.connect(),again=once(second,'data');second.write('fresh');assert.equal((await again)[0].toString(),'fresh');
 assert.equal(deadlines,2);second.destroy();
});

test('normal upstream EOF delivers queued bytes before the client is retired',{timeout:5000},async t=>{
 const payload=Buffer.alloc(256*1024,0x6b);
 const f=await fixture(t,socket=>socket.once('data',()=>socket.end(payload)));
 const client=await f.connect(),chunks=[];client.on('data',chunk=>chunks.push(chunk));
 const closed=new Promise(resolve=>client.once('close',resolve));client.write('send');await bounded(closed);
 assert.deepEqual(Buffer.concat(chunks),payload);
});

test('client retirement closes its upstream and an upstream reset closes its client',{timeout:5000},async t=>{
 const accepted=[];const f=await fixture(t,socket=>accepted.push(socket));
 const client=await f.connect(),peer=accepted[0],upstreamClosed=new Promise(resolve=>peer.once('close',resolve));
 client.destroy();await bounded(upstreamClosed);assert.equal(peer.destroyed,true);
 const next=await f.connect(),nextClosed=new Promise(resolve=>next.once('close',resolve));accepted[1].resetAndDestroy();
 await bounded(nextClosed);assert.equal(next.destroyed,true);
});
