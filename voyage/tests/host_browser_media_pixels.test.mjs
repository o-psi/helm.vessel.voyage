import assert from 'node:assert/strict';
import vm from 'node:vm';
import {test} from 'node:test';
import {readOwnedMediaJpegs,mediaMovementFacts} from './host_browser_media_pixels.mjs';

// Exercise the serialized function used by the read-only DOM evaluator, including
// its actual expected-identity boundary, without a browser or real-time waits.
function observe(expected){
 const names=['canvas','video','unsupported'];
 const images=names.map((name,index)=>({
  style:{left:String(index*20),top:'0',width:'10',height:'10'},
  dataset:{nodeId:String(index+1),version:'1'},isConnected:true,
  src:'data:image/jpeg;base64,AA==',complete:true,naturalWidth:10,naturalHeight:10
 }));
 const mediaDocument={title:'Media fixture',querySelector(selector){
  if(selector==='h1')return {textContent:'Media qualification fixture'};
  if(selector==='#video-state')return {dataset:{state:'playing',frames:'1',timeMs:'1',starts:'1'}};
  const index=['#canvas','#video','#unsupported-frame'].indexOf(selector);
  return index<0?null:{getBoundingClientRect:()=>({left:index*20,top:0,width:10,height:10})};
 }};
 const frame={contentDocument:mediaDocument};
 const root={isConnected:true,dataset:{state:'watching',browserId:'browser-a',browserIncarnation:'incarnation-a',browserDocumentEpoch:'2',browserCaptureEpoch:'3'},
  querySelector:()=>frame,querySelectorAll:()=>images};
 const context=vm.createContext({document:{querySelector:()=>root},performance:{now:()=>0},setTimeout:resolve=>resolve(),expected});
 return vm.runInContext(`(${readOwnedMediaJpegs.toString()})({expected})`,context);
}
function identity(){return {fence:{browser_id:'browser-a',incarnation:'incarnation-a',document_epoch:2,capture_epoch:3},ids:{canvas:1,video:2,unsupported:3}};}

test('serialized DOM observer accepts transport-reordered identity keys',async()=>{
 const original=identity();
 const reordered={ids:{unsupported:3,video:2,canvas:1},fence:{capture_epoch:3,document_epoch:2,incarnation:'incarnation-a',browser_id:'browser-a'}};
 for(const expected of [null,original,reordered]){
  const result=await observe(expected);
  assert.equal(result.same_dom_nodes,true);
  assert.equal(result.samples.length,12);
  assert.deepEqual(JSON.parse(JSON.stringify(result.identity)),original);
 }
});

test('serialized DOM observer refuses changed, missing, extra and mistyped identity fields',async()=>{
 const mutations=[
  value=>{value.fence.browser_id='browser-b';},
  value=>{value.fence.incarnation='incarnation-b';},
  value=>{value.fence.document_epoch=4;},
  value=>{value.fence.capture_epoch=4;},
  value=>{value.ids.canvas=9;},
  value=>{value.ids.video='2';},
  value=>{value.fence.document_epoch='2';},
  value=>{delete value.fence.incarnation;},
  value=>{delete value.ids.unsupported;},
  value=>{delete value.ids;},
  value=>{value.fence.extra=true;},
  value=>{value.ids.extra=4;},
  value=>{value.extra=true;},
  value=>{value.fence=[];},
  value=>{value.ids=null;}
 ];
 for(const mutate of mutations){const value=identity();mutate(value);await assert.rejects(observe(value),/owned public media observation unavailable/);}
 for(const value of [false,0,'identity',[]])await assert.rejects(observe(value),/owned public media observation unavailable/);
});

function movingSamples(){
 return [0,500,0].map((time_ms,index)=>({
  canvas:{version:index+1,color:index===1?'blue':'orange'},
  video:{version:index+1,color:index===1?'blue':'orange'},
  playback:{state:'playing',starts:1,frames:index+10,time_ms}
 }));
}
test('looping media can advance time between equal endpoint timestamps',()=>{
 const facts=mediaMovementFacts(movingSamples());
 assert.equal(facts.playback.media_time_changed,true);
 assert.equal(facts.playback.decoded_frames_before,10);
 assert.equal(facts.playback.decoded_frames_after,12);
});
test('media movement refuses stalled, regressing or invalid playback traces',()=>{
 const mutations=[
  values=>values.forEach(value=>{value.playback.frames=10;}),
  values=>values.forEach(value=>{value.playback.time_ms=0;}),
  values=>{values[1].playback.frames=9;},
  values=>{values[1].playback.state='paused';},
  values=>{values[1].playback.starts=2;},
  values=>{values[1].playback.time_ms=-1;}
 ];
 for(const mutate of mutations){const values=movingSamples();mutate(values);assert.throws(()=>mediaMovementFacts(values));}
});
