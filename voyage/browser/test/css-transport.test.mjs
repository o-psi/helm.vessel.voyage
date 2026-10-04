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
