import test from 'node:test';
import assert from 'node:assert/strict';
import {gzipSync} from 'node:zlib';
import {decodeBrowserMirror,BrowserConnection} from '../../helm/browser-view/viewer.mjs';
const limit=8*1024*1024;
const packed=(events,css_dictionary=[])=>{
 const raw=Buffer.from(JSON.stringify({events,css_dictionary}));const chunks=[];
 for(let i=0;i<raw.length;i+=512*1024)chunks.push(gzipSync(raw.subarray(i,i+512*1024)).toString('base64'));
 return {encoding:'gzip-chunks',format:'css_chunks_v1',chunks,total_bytes:raw.length,cursor:1,reset:false,frames:[]};
};
const legacy=events=>({encoding:'gzip',data_base64:gzipSync(JSON.stringify(events)).toString('base64'),cursor:1,reset:false,frames:[]});
const cssNode=index=>({type:2,id:index+1,tagName:'style',attributes:{_cssText:{$css:0}},childNodes:[]});
test('dictionary chunks preserve a snapshot beyond legacy decoded limit, including UTF8 split boundaries',async()=>{
 const css='/*λ😀*/body{color:red}\n'.repeat(100000),events=[{type:2,data:{node:{childNodes:[cssNode(1),cssNode(2)]}}}];
 assert.ok(Buffer.byteLength(JSON.stringify({events,css_dictionary:[css]}))>512*1024);
 const result=await decodeBrowserMirror(packed(events,[css]));
 assert.equal(result[0].data.node.childNodes[0].attributes._cssText,css);
 assert.equal(result[0].data.node.childNodes[1].attributes._cssText,css);
 await assert.rejects(decodeBrowserMirror(legacy(result)),/mirror_limit/);
});
test('rejects missing chunks, oversized stream, malformed refs and unauthorized ref locations',async()=>{
 const packet=packed([{type:2,data:{node:cssNode(1)}}],['a{}']);
 await assert.rejects(decodeBrowserMirror(packed([null])),/dictionary/);
 await assert.rejects(decodeBrowserMirror({...packet,total_bytes:packet.total_bytes+1}),/chunk_size/);
 await assert.rejects(decodeBrowserMirror({...packet,chunks:[]}),/chunks/);
 await assert.rejects(decodeBrowserMirror({...packet,chunks:Array(17).fill(packet.chunks[0])}),/chunks/);
 const huge=gzipSync(Buffer.alloc(512*1024+1,32)).toString('base64');
 await assert.rejects(decodeBrowserMirror({...packet,chunks:[huge],total_bytes:512*1024+1}),/mirror_limit/);
 await assert.rejects(decodeBrowserMirror(packed([{attributes:{_cssText:{$css:5}}}],['a'])),/reference/);
 await assert.rejects(decodeBrowserMirror(packed([{secret:{$css:0}}],['a'])),/reference_location/);
});
test('charges compact and expanded budget across top and child frames',async()=>{
 const budget={remaining:limit,compactRemaining:limit};
 const packet=packed([{attributes:{_cssText:{$css:0}}}],['x'.repeat(5*1024*1024)]);
 await decodeBrowserMirror(packet,budget);
 await assert.rejects(decodeBrowserMirror(packet,budget),/mirror/);
 const css='x'.repeat(3*1024*1024);
 await assert.rejects(decodeBrowserMirror(packed([cssNode(1),cssNode(2),cssNode(3)],[css])),/mirror_limit/);
});
const binding={incarnation:'owner',browser_id:'browser',attachment_id:'attachment',tab_id:'tab',document_epoch:1,capture_epoch:1};
const status=formats=>({available:true,running:true,binding,mirror_formats:formats});
function fixture(value,formats){
 const operations=[],replayed=[];
 const connection=new BrowserConnection({mirror:{replaceChildren(){}},transport:async op=>{operations.push(op);return {status:status(formats),value};}});
 connection.status=status(formats);connection.replayer={addEvent:event=>replayed.push(event),destroy(){}};
 return {connection,operations,replayed};
}
test('negotiates only advertised formats and rejects any bad child before top replay',async()=>{
 const events=[{type:3,data:{source:0,texts:[]}}];
 const old=fixture(legacy(events),undefined);await old.connection.pull();assert.equal(old.operations[0].format,undefined);
 const current=fixture(packed(events),['css_chunks_v1']);await current.connection.pull();assert.equal(current.operations[0].format,'css_chunks_v1');
 const invalid=fixture({...packed(events),frames:[{frame_id:'child',host_node_id:3,...packed(events),total_bytes:1}]},['css_chunks_v1']);
 await assert.rejects(invalid.connection.pull());assert.equal(invalid.replayed.length,0);assert.equal(invalid.connection.cursor,0);
});
test('capture changes or disconnect during inflation discard the complete packet',async()=>{
 const f=fixture(packed([{type:3,data:{source:0,texts:[]}}]),['css_chunks_v1']);
 const pending=f.connection.pull();f.connection.epoch++;await pending.catch(()=>{});
 assert.equal(f.replayed.length,0);assert.equal(f.connection.cursor,0);
});
test('snapshot policy validation happens before replay and preserves ordinary attribute names',async()=>{
 const current=fixture(packed([{type:2,data:{node:{childNodes:[]}}}]),['css_chunks_v1']);
 await assert.rejects(current.connection.pull(),/invalid_full_snapshot/);assert.equal(current.replayed.length,0);
 const node=JSON.parse('{"attributes":{"constructor":"ordinary","__proto__":"ordinary","_cssText":{"$css":0}}}');
 const [decoded]=await decodeBrowserMirror(packed([node],['a{}']));
 assert.equal(decoded.attributes.constructor,'ordinary');assert.equal(decoded.attributes.__proto__,'ordinary');
});
