import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {runInNewContext} from 'node:vm';
import {createCssCodec} from '../css-transport.mjs';
import {decodeCssMirror,encodeCssMirror,Worker} from '../worker.mjs';
import {decodeBrowserMirror} from '../../../helm/browser-view/viewer.mjs';
const codec=createCssCodec();
const normalize=value=>JSON.parse(JSON.stringify(value));
const css=(size,symbol='a')=>'body{--fixture:"'+symbol.repeat(size)+'";}';
const full=()=>({type:2,data:{node:{type:0,id:1,childNodes:[{type:2,id:2,tagName:'html',attributes:{},childNodes:[{type:2,id:3,tagName:'head',attributes:{},childNodes:[...Array.from({length:5},(_,index)=>({type:2,id:index+4,tagName:'link',attributes:{href:`https://fixture.test/${index}/style.css`,_cssText:css(330215)},childNodes:[]})),{type:2,id:10,tagName:'style',attributes:{},childNodes:[{type:3,id:11,textContent:css(756433,'é')}]},{type:2,id:12,tagName:'link',attributes:{_cssText:css(458798)},childNodes:[]}]},{type:2,id:13,tagName:'body',attributes:{style:'color:red'},childNodes:[{type:3,id:14,textContent:'Visible body text'}]}]}]}}});
async function recorder({Compressor=CompressionStream,snapshot=full()}={}){
 let emit,stopped=0;
 const record=options=>{emit=options.emit;emit({type:4,data:{href:'https://fixture.test/'}});emit(snapshot);return ()=>{stopped++;};};
 record.takeFullSnapshot=()=>emit(snapshot);
 const source=(await readFile(new URL('../mirror-source.mjs',import.meta.url),'utf8')).replace('__VOYAGE_CAPTURE_KEY__','fixture-key').replaceAll('__VOYAGE_CSS_CODEC__',`(${createCssCodec.toString()})`);
 const scope={rrweb:{record},TextEncoder,Blob,Response,CompressionStream:Compressor,btoa};
 runInNewContext(source,scope);
 return {mirror:scope.__voyageMirror,emit:event=>emit(event),stopped:()=>stopped};
}
test('large CSS snapshot round trips losslessly through recorder, worker and viewer',async()=>{
 const expected=full(),r=await recorder({snapshot:expected});r.mirror.enable('fixture-key',1);
 assert.equal(r.mirror.drain(0,2200000,'fixture-key',1).error,'page_too_large');
 const wire=await r.mirror.drainCss(0,2200000,'fixture-key',1);
 assert.equal(wire.encoding,'gzip-chunks');assert.ok(wire.chunks.length>1);assert.ok(JSON.stringify(wire).length<2200000);
 const restored=decodeCssMirror(normalize(wire));assert.deepEqual(restored.events.at(-1),expected);
 const consumer=await decodeBrowserMirror(normalize(wire));assert.deepEqual(consumer.at(-1),expected);
 const repeated=await r.mirror.drainCss(0,2200000,'fixture-key',1);
 assert.deepEqual(decodeCssMirror(normalize(repeated)).events,restored.events);
 // Cursor reads for another viewer/reset do not consume the queued snapshot.
 assert.equal(repeated.cursor,wire.cursor);
 assert.equal(r.mirror.drain(0,2200000,'fixture-key',1).error,'page_too_large');
 const incremental={type:3,data:{source:8,id:3,adds:[{rule:'a{color:blue}',index:0}],replaceSync:'b{color:red}'}};
 r.emit(incremental);
 const next=await r.mirror.drainCss(wire.cursor,2200000,'fixture-key',1);
 assert.deepEqual(decodeCssMirror(normalize(next)).events,[incremental]);
 assert.equal(next.reset,false);
 const stopped=r.mirror.stop('fixture-key',2);assert.equal(stopped.pending_events,0);assert.equal(stopped.pending_bytes,0);
 assert.equal((await r.mirror.drainCss(0,2200000,'fixture-key',1)).error,'recorder_disabled');
});
test('dictionary includes only exact referenced CSS and preserves URL bases independently',()=>{
 const worker=new Worker();
 const events=[{type:2,data:{node:{type:2,id:1,tagName:'link',attributes:{href:'https://fixture.test/a/style.css',_cssText:'body{background:url(logo.png)}'},childNodes:[]}}}];
 worker.assets.set('https://fixture.test/a/logo.png','data:image/png;base64,QQ==');
 worker.assets.set('https://fixture.test/b/logo.png','data:image/png;base64,Qg==');
 const a=normalize(events),b=normalize(events);b[0].data.node.attributes.href='https://fixture.test/b/style.css';
 worker.inlineAssets(a,'https://fixture.test/a/');worker.inlineAssets(b,'https://fixture.test/b/');
 assert.notEqual(a[0].data.node.attributes._cssText,b[0].data.node.attributes._cssText);
 assert.deepEqual(decodeCssMirror(encodeCssMirror(a).value).events,a);
 assert.deepEqual(decodeCssMirror(encodeCssMirror(b).value).events,b);
 const packed=codec.pack([full()]);assert.ok(packed.payload.css_dictionary.length<8);
 assert.deepEqual(codec.unpack(packed.payload).events,[full()]);
});
test('compression remains an observed obligation until generation-fenced completion',async()=>{
 let release,started;const gate=new Promise(resolve=>{release=resolve;});const began=new Promise(resolve=>{started=resolve;});
 class PausedCompressor{constructor(){const actual=new CompressionStream('gzip');const pause=new TransformStream({async transform(chunk,controller){started();await gate;controller.enqueue(chunk);}});this.writable=pause.writable;this.readable=pause.readable.pipeThrough(actual);}}
 const r=await recorder({Compressor:PausedCompressor,snapshot:{type:2,data:{node:{type:0,id:1,childNodes:[]}}}});
 r.mirror.enable('fixture-key',1);const pending=r.mirror.drainCss(0,2200000,'fixture-key',1);await began;
 const stopped=r.mirror.stop('fixture-key',2);assert.ok(stopped.pending_events>0);assert.ok(stopped.pending_bytes>0);
 release();assert.equal((await pending).error,'capture_fenced');
 const observed=r.mirror.stop('fixture-key',2);assert.equal(observed.pending_events,0);assert.equal(observed.pending_bytes,0);
});
test('codec and chunks refuse malformed, unused, oversized and misplaced data',()=>{
 const packet=encodeCssMirror([{type:2,data:{node:{type:2,id:1,tagName:'style',attributes:{},childNodes:[{type:3,id:2,textContent:'a{}'}]}}}]).value;
 assert.throws(()=>decodeCssMirror({...packet,total_bytes:packet.total_bytes+1}));
 assert.throws(()=>decodeCssMirror({...packet,chunks:Array(17).fill(packet.chunks[0])}));
 assert.throws(()=>decodeCssMirror(packet,1));
 assert.throws(()=>codec.unpack({events:[{type:3,data:{text:{$css:0}}}],css_dictionary:['secret']}));
 assert.throws(()=>codec.unpack({events:[],css_dictionary:['unused']}));
 assert.throws(()=>codec.unpack({events:[{type:3,data:{rule:{$css:1}}}],css_dictionary:['a{}']}));
 assert.throws(()=>codec.pack([{type:2,data:{node:{type:2,id:1,tagName:'style',attributes:{_cssText:'x'.repeat(codec.limit)},childNodes:[]}}}]));
 assert.equal(new Worker().status().mirror_formats[0],'css_chunks_v1');
});

test('worker applies negotiated format independently and refuses aggregate frame excess atomically',async()=>{
 const worker=new Worker(),viewerA='11111111-2222-4333-8444-555555555555',viewerB='aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee';
 const main={},child={parentFrame:()=>main,frameElement:async()=>({evaluate:async()=>15,dispose:async()=>{}}),url:()=> 'https://fixture.test/child/'};
 worker.task={};worker.page={isClosed:()=>false,mainFrame:()=>main,frames:()=>[main,child],url:()=> 'https://fixture.test/'};
 for(const viewer of [viewerA,viewerB])worker.viewers.set(viewer,{seq:0,mirrorCursor:0,frameCursors:new Map()});
 worker.captureVisuals=async()=>[];worker.captureFrameVisuals=async()=>[];
 let childEvents=[full()];
 worker.recorderCall=async frame=>({...encodeCssMirror(frame===main?[full()]:childEvents).value,cursor:9,reset:true,latest:9});
 const a=await worker.mirror({viewer:viewerA,since:0,format:'css_chunks_v1'});
 const b=await worker.mirror({viewer:viewerB,since:0,format:'css_chunks_v1'});
 assert.deepEqual(a,b);assert.equal(a.frames.length,1);assert.equal(a.cursor,9);
 assert.equal(decodeCssMirror(a).events.length,1);assert.equal(decodeCssMirror(a.frames[0]).events.length,1);
 const prior=worker.viewers.get(viewerA).mirrorCursor;
 childEvents=[{type:2,data:{node:{type:2,id:1,tagName:'style',attributes:{_cssText:css(6*1024*1024)},childNodes:[]}}}];
 await assert.rejects(worker.mirror({viewer:viewerA,since:9,format:'css_chunks_v1'}),error=>error.code==='mirror_limit');
 assert.equal(worker.viewers.get(viewerA).mirrorCursor,prior);
 await assert.rejects(worker.mirror({viewer:viewerA,since:0,format:'invented'}),error=>error.code==='unsupported_format');
});

test('compact batch merge reindexes CSS losslessly and charges exact logical bytes',()=>{
 const original=[{type:4,data:{href:'https://fixture.test/'}},full(),{type:3,data:{source:8,adds:[{rule:'a{color:red}',index:0}],replaceSync:'b{color:blue}'}}];
 const packets=original.map(event=>{const packed=codec.pack([event]);return {...packed.payload,expanded_bytes:packed.expanded_bytes};});
 const before=JSON.stringify(packets),merged=codec.combine(packets);
 assert.equal(merged.expanded_bytes,codec.pack(original).expanded_bytes);
 assert.deepEqual(codec.unpack(merged.payload).events,original);
 assert.equal(JSON.stringify(packets),before);
});
test('two cold readers reuse one immutable encoding and all document fences invalidate it',async()=>{
 let compressions=0;
 class CountedCompressor{constructor(){compressions++;return new CompressionStream('gzip');}}
 const r=await recorder({Compressor:CountedCompressor});r.mirror.enable('fixture-key',1);
 const [a,b]=await Promise.all([r.mirror.drainCss(0,2200000,'fixture-key',1),r.mirror.drainCss(0,2200000,'fixture-key',1)]);
 assert.equal(a,b);assert.equal(compressions,a.chunks.length);assert.ok(Object.isFrozen(a)&&Object.isFrozen(a.chunks));
 const decoded=decodeCssMirror(normalize(a));
 r.emit({type:3,data:{source:8,adds:[{rule:'a{color:red}',index:0}]}});
 const changed=await r.mirror.drainCss(0,2200000,'fixture-key',1);assert.notEqual(changed,a);assert.equal(decodeCssMirror(normalize(changed)).events.length,decoded.events.length+1);
 const checkout=await r.mirror.drainCss(Number.MAX_SAFE_INTEGER,2200000,'fixture-key',1);assert.notEqual(checkout,changed);
 r.mirror.enable('fixture-key',2);const generation=await r.mirror.drainCss(0,2200000,'fixture-key',2);assert.notEqual(generation,checkout);
 const stopped=r.mirror.stop('fixture-key',3);assert.equal(stopped.pending_bytes,0);assert.equal(stopped.pending_events,0);
 assert.equal((await r.mirror.drainCss(0,2200000,'fixture-key',2)).error,'recorder_disabled');
 r.mirror.enable('fixture-key',4);const restarted=await r.mirror.drainCss(0,2200000,'fixture-key',4);assert.notEqual(restarted,generation);
});
test('worker rewritten cache is exact, immutable, asset-sensitive and retired at capture stop',async()=>{
 const worker=new Worker(),frame={},value=encodeCssMirror([full()]).value;let rewrites=0;
 const original=worker.inlineAssets.bind(worker);worker.inlineAssets=(...args)=>{rewrites++;return original(...args);};
 const a=worker.prepareCssBatch(value,frame,'https://fixture.test/',0,1,codec.limit);
 const b=worker.prepareCssBatch(value,frame,'https://fixture.test/',0,1,codec.limit);
 assert.equal(a,b);assert.equal(rewrites,1);assert.ok(Object.isFrozen(a.value.chunks));
 worker.assetRevision++;const changed=worker.prepareCssBatch(value,frame,'https://fixture.test/',0,1,codec.limit);assert.notEqual(a,changed);assert.equal(rewrites,2);
 worker.advance('control','capture');assert.equal(worker.cssBatchCache.size,0);assert.equal(worker.cssCacheBytes,0);
 worker.prepareCssBatch(value,frame,'https://fixture.test/',worker.captureActivityGeneration,worker.epochs.capture,codec.limit);
 worker.publishCaptureObservation=async()=>true;await worker.stopMirrors();assert.equal(worker.cssBatchCache.size,0);assert.equal(worker.cssCacheBytes,0);
});
test('only observation-unavailable reads retry within original bounded read and authority guard',async()=>{
 const worker=new Worker(),frame={};let reads=0,guards=0;
 worker.recorderCall=async()=>{reads++;if(reads===1)throw {code:'observation_unavailable'};return {ok:true};};
 assert.deepEqual(await worker.mirrorRead(frame,0,2200000,0,'css_chunks_v1',100,()=>{guards++;}),{ok:true});assert.equal(reads,2);assert.ok(guards>=4);
 reads=0;worker.recorderCall=async()=>{reads++;throw {code:'operation_timeout'};};
 await assert.rejects(worker.mirrorRead(frame,0,2200000,0,'css_chunks_v1',100,()=>{}),error=>error.code==='operation_timeout');assert.equal(reads,1);
 reads=0;let allowed=true;worker.recorderCall=async()=>{reads++;allowed=false;throw {code:'observation_unavailable'};};
 const authority=()=>{if(!allowed)throw {code:'capture_fenced'};};
 await assert.rejects(worker.mirrorRead(frame,0,2200000,0,'css_chunks_v1',100,authority,authority),error=>error.code==='capture_fenced');assert.equal(reads,1);
 worker.recorderCall=async()=>{throw {code:'observation_unavailable'};};
 const began=Date.now();await assert.rejects(worker.mirrorRead(frame,0,2200000,0,'css_chunks_v1',40,()=>{}),error=>['observation_unavailable','operation_timeout'].includes(error.code));assert.ok(Date.now()-began<100);
});


test('a concurrent capture fence never masks timeout or retires unknown capture work',async()=>{
 const worker=new Worker();let reads=0,fenced=false;
 worker.recorderCall=async()=>{reads++;fenced=true;throw {code:'operation_timeout'};};
 await assert.rejects(worker.mirrorRead({},0,2200000,0,'css_chunks_v1',100,()=>{if(fenced)throw {code:'capture_fenced'};}),error=>error.code==='operation_timeout');
 assert.equal(reads,1);assert.equal(worker.captureTaskUncertain,true);
});

test('exact concurrent CSS readers share one immutable recorder producer with independent authority',async()=>{
 const worker=new Worker(),frame={},other={};let release,reads=0,first=true;
 worker.recorderCall=async()=>{reads++;await new Promise(resolve=>{release=resolve;});return {encoding:'gzip-chunks',chunks:['encoded'],cursor:1};};
 const one=worker.mirrorRead(frame,0,2200000,0,'css_chunks_v1',1000,()=>{if(!first)throw {code:'private'};},()=>{});
 const two=worker.mirrorRead(frame,0,2200000,0,'css_chunks_v1',1000,()=>{},()=>{});
 await new Promise(resolve=>setTimeout(resolve,0));assert.equal(reads,1);first=false;release();
 await assert.rejects(one,error=>error.code==='private');const value=await two;
 assert.ok(Object.isFrozen(value)&&Object.isFrozen(value.chunks));assert.equal(worker.cssReads.size,0);
 worker.recorderCall=async()=>{reads++;return {chunks:['new']};};
 await Promise.all([worker.mirrorRead(frame,1,2200000,0,'css_chunks_v1',1000,()=>{}),worker.mirrorRead(other,1,2200000,0,'css_chunks_v1',1000,()=>{})]);assert.equal(reads,3);
});
test('CSS producer retirement remains owned after caller deadline and capture fence',async()=>{
 const worker=new Worker(),frame={};let release,reads=0;
 worker.recorderCall=async()=>{reads++;await new Promise(resolve=>{release=resolve;});return {chunks:['encoded']};};
 const pending=worker.mirrorRead(frame,0,2200000,0,'css_chunks_v1',20,()=>{},()=>{});
 await assert.rejects(pending,error=>error.code==='operation_timeout');assert.equal(reads,1);assert.equal(worker.cssReads.size,1);assert.ok(worker.captureTasks.size>=1);assert.equal(worker.captureTaskUncertain,true);
 worker.captureActivityGeneration++;release();await Promise.allSettled([...worker.captureTasks]);assert.equal(worker.cssReads.size,0);assert.equal(worker.captureTasks.size,0);
});
test('CSS producer keys refuse generation/private changes and bound outstanding distinct reads',async()=>{
 const worker=new Worker();let release;const waiting=new Promise(resolve=>{release=resolve;});worker.recorderCall=async()=>{await waiting;return {chunks:['encoded']};};
 const tasks=Array.from({length:4},()=>worker.mirrorRead({},0,2200000,0,'css_chunks_v1',1000,()=>{},()=>{}));
 await assert.rejects(worker.mirrorRead({},0,2200000,0,'css_chunks_v1',1000,()=>{}),error=>error.code==='capture_busy');
 await assert.rejects(worker.mirrorRead({},0,2200000,0,'css_chunks_v1',1000,()=>{throw {code:'private'};}),error=>error.code==='private');
 worker.captureActivityGeneration++;release();for(const task of tasks)await assert.rejects(task,error=>error.code==='capture_fenced');assert.equal(worker.cssReads.size,0);
});
test('legacy recorder reads stay independent rather than sharing mutable events',async()=>{
 const worker=new Worker(),frame={};let calls=0;worker.recorderCall=async()=>{calls++;return {events:[]};};
 const values=await Promise.all([worker.mirrorRead(frame,0,1000,0,null,1000,()=>{}),worker.mirrorRead(frame,0,1000,0,null,1000,()=>{})]);assert.equal(calls,2);assert.notEqual(values[0],values[1]);
});
test('in-flight CSS key separates exact cursor and budget while legacy never joins',async()=>{
 const worker=new Worker(),frame={};let release,calls=0;const waiting=new Promise(resolve=>{release=resolve;});worker.recorderCall=async()=>{calls++;await waiting;return {chunks:['encoded']};};
 const reads=[worker.mirrorRead(frame,0,2200000,0,'css_chunks_v1',1000,()=>{}),worker.mirrorRead(frame,1,2200000,0,'css_chunks_v1',1000,()=>{}),worker.mirrorRead(frame,0,550000,0,'css_chunks_v1',1000,()=>{}),worker.mirrorRead(frame,0,2200000,0,null,1000,()=>{})];
 await new Promise(resolve=>setTimeout(resolve,0));assert.equal(calls,4);assert.equal(worker.cssReads.size,3);release();await Promise.all(reads);assert.equal(worker.cssReads.size,0);
});
test('CSS sharing refuses mutable nested envelopes and expired callers before producer admission',async()=>{
 const worker=new Worker();let reads=0;worker.recorderCall=async()=>{reads++;return {chunks:['encoded'],nested:{events:[]}};};
 await assert.rejects(worker.mirrorRead({},0,2200000,0,'css_chunks_v1',1000,()=>{}),error=>error.code==='mirror_limit');assert.equal(worker.cssReads.size,0);
 const before=reads,now=Date.now;let clock=1000;Date.now=()=>clock;
 try{await assert.rejects(worker.mirrorRead({},0,2200000,0,'css_chunks_v1',1,()=>{clock=1002;}),error=>error.code==='observation_unavailable');assert.equal(reads,before);assert.equal(worker.cssReads.size,0);}finally{Date.now=now;}
});
