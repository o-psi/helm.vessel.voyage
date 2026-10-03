// Synthetic /media JPEG reduction only. No DOM/network/filesystem or new decoder dependency.
import assert from 'node:assert/strict';
export function mediaColor(pixels){
 assert.equal(pixels.length,64,'exact decoded center4x4 RGBA sample required');
 const rgb=[0,0,0];
 for(let i=0;i<64;i+=4){for(let c=0;c<4;c++)assert.ok(Number.isInteger(pixels[i+c])&&pixels[i+c]>=0&&pixels[i+c]<=255);
  assert.equal(pixels[i+3],255);for(let c=0;c<3;c++)rgb[c]+=pixels[i+c]/16;}
 if(rgb[0]>=180&&rgb[1]>=100&&rgb[1]<=210&&rgb[2]<=80)return 'orange';
 if(rgb[0]<=80&&rgb[1]<=80&&rgb[2]>=180)return 'blue';
 return 'unqualified';
}
export function decodeMediaJpeg(sample,decoder){
 assert.ok(Number.isSafeInteger(sample.version)&&sample.version>0);
 assert.ok(typeof sample.src==='string'&&sample.src.length<=270000&&/^data:image\/jpeg;base64,[A-Za-z0-9+/]+={0,2}$/.test(sample.src));
 const raw=sample.src.slice('data:image/jpeg;base64,'.length),bytes=Buffer.from(raw,'base64');
 assert.ok(bytes.length>4&&bytes.length<=200000&&bytes.toString('base64')===raw);
 assert.equal(bytes.readUInt16BE(0),0xffd8);
 const image=decoder.decode(bytes,{useTArray:true,formatAsRGBA:true,tolerantDecoding:false,maxResolutionInMP:0.25,maxMemoryUsageInMB:16});
 assert.ok(image.width>=8&&image.height>=8&&image.width<=512&&image.height<=512);
 const pixels=[];const left=Math.floor(image.width/2)-2,top=Math.floor(image.height/2)-2;
 for(let y=top;y<top+4;y++)for(let x=left;x<left+4;x++){const offset=(y*image.width+x)*4;for(let c=0;c<4;c++)pixels.push(image.data[offset+c]);}
 return {version:sample.version,color:mediaColor(pixels),width:image.width,height:image.height};
}
export function mediaMovementFacts(samples){
 assert.ok(Array.isArray(samples)&&samples.length>=2&&samples.length<=24);
 const facts={};
 for(const name of ['canvas','video']){
  const values=samples.map(sample=>sample[name]);
  for(let i=0;i<values.length;i++)assert.ok(Number.isSafeInteger(values[i].version)&&values[i].version>0&&(!i||values[i].version>=values[i-1].version));
  const colored=values.filter(value=>['orange','blue'].includes(value.color));
  assert.equal(new Set(colored.map(value=>value.color)).size,2,`${name} actual decoded content did not change between both synthetic colors`);
  assert.ok(new Set(colored.map(value=>value.version)).size>=2);
  facts[name]={decoded_colors:['orange','blue'],first_version:values[0].version,last_version:values.at(-1).version,content_changed:true};
 }
 const first=samples[0].playback,last=samples.at(-1).playback;
 const playback=samples.map(sample=>sample.playback);
 for(let index=0;index<playback.length;index++){
  const value=playback[index];
  assert.ok(value.state==='playing'&&value.starts===1&&Number.isSafeInteger(value.frames)&&value.frames>0&&Number.isSafeInteger(value.time_ms)&&value.time_ms>=0);
  assert.ok(!index||value.frames>=playback[index-1].frames,'decoded playback frame count regressed');
 }
 // The fixture video loops, so endpoint media times may alias. Its sampled
 // trace must contain an actual time change while decoded frames advance.
 assert.ok(last.frames>first.frames&&playback.some(value=>value.time_ms!==first.time_ms),'decoded playback callbacks/time did not advance');
 return {...facts,playback:{decoded_frames_before:first.frames,decoded_frames_after:last.frames,media_time_changed:true,one_play_intent:true},samples:samples.length};
}

// For the documented CUA read-only DOM evaluator. Keep returned JPEGs private in
// Node; decode with the already pinned local decoder and publish reduced facts only.
export async function readOwnedMediaJpegs({expected=null}={}){
 const fail=()=>{throw Error('owned public media observation unavailable');};
 const root=document.querySelector('.host-browser-viewer');
 if(!root||!['agent','human','watching'].includes(root.dataset.state))fail();
 const fence=()=>({browser_id:root.dataset.browserId,incarnation:root.dataset.browserIncarnation,
  document_epoch:Number(root.dataset.browserDocumentEpoch),capture_epoch:Number(root.dataset.browserCaptureEpoch)});
 const initial=fence();if(!initial.browser_id||!initial.incarnation||!Number.isSafeInteger(initial.document_epoch)||initial.document_epoch<=0||!Number.isSafeInteger(initial.capture_epoch)||initial.capture_epoch<=0)fail();
 const frame=root.querySelector('.browser-next-mirror iframe'),mediaDocument=frame?.contentDocument;
 if(!mediaDocument||mediaDocument.title!=='Media fixture'||mediaDocument.querySelector('h1')?.textContent!=='Media qualification fixture')fail();
 const selectors={canvas:'#canvas',video:'#video',unsupported:'#unsupported-frame'};
 const selected=()=>{
  const images=[...root.querySelectorAll('.browser-next-visuals > img.browser-next-visual')];if(images.length!==3)fail();
  const found={};
  for(const [name,selector] of Object.entries(selectors)){
   const target=mediaDocument.querySelector(selector);if(!target)fail();const r=target.getBoundingClientRect();
   const matching=images.filter(image=>Math.abs(parseFloat(image.style.left)-Math.max(0,r.left))<2&&Math.abs(parseFloat(image.style.top)-Math.max(0,r.top))<2&&Math.abs(parseFloat(image.style.width)-r.width)<2&&Math.abs(parseFloat(image.style.height)-r.height)<2);
   if(matching.length!==1)fail();found[name]=matching[0];
  }
  if(new Set(Object.values(found)).size!==3)fail();return found;
 };
 const nodes=selected(),ids=Object.fromEntries(Object.entries(nodes).map(([name,image])=>[name,Number(image.dataset.nodeId)]));
 if(Object.values(ids).some(id=>!Number.isSafeInteger(id)||id<=0)||new Set(Object.values(ids)).size!==3)fail();
 // The DOM evaluator may reorder object keys while transporting this identity.
 // Require its exact shape and scalar values without relying on insertion order.
 const exactFields=(value,reference)=>value!==null&&typeof value==='object'&&!Array.isArray(value)
  &&Object.keys(value).length===Object.keys(reference).length
  &&Object.entries(reference).every(([key,field])=>Object.prototype.hasOwnProperty.call(value,key)&&value[key]===field);
 if(expected!==null&&(!expected||typeof expected!=='object'||Array.isArray(expected)
  ||Object.keys(expected).length!==2||!Object.prototype.hasOwnProperty.call(expected,'fence')
  ||!Object.prototype.hasOwnProperty.call(expected,'ids')||!exactFields(expected.fence,initial)||!exactFields(expected.ids,ids)))fail();
 const result=[],deadline=performance.now()+10000;
 for(let index=0;index<12;index++){
  if(index)await new Promise(resolve=>setTimeout(resolve,650));
  if(!root.isConnected||!['agent','human','watching'].includes(root.dataset.state)||frame.contentDocument!==mediaDocument||JSON.stringify(fence())!==JSON.stringify(initial))fail();
  const current=selected(),row={};if(performance.now()>=deadline)fail();
  for(const name of Object.keys(selectors)){
   const image=nodes[name];if(current[name]!==image||!image.isConnected||Number(image.dataset.nodeId)!==ids[name]||image.src.length>270000||!image.src.startsWith('data:image/jpeg;base64,'))fail();
   while(!image.complete||image.naturalWidth<8||image.naturalHeight<8){
    if(performance.now()>=deadline||!image.isConnected||JSON.stringify(fence())!==JSON.stringify(initial))fail();
    await new Promise(resolve=>setTimeout(resolve,25));
   }
   const version=Number(image.dataset.version);if(!Number.isSafeInteger(version)||version<=0)fail();row[name]={version,src:image.src};
  }
  const marker=mediaDocument.querySelector('#video-state');if(!marker)fail();
  row.playback={state:marker.dataset.state,frames:Number(marker.dataset.frames),time_ms:Number(marker.dataset.timeMs),starts:Number(marker.dataset.starts)};
  result.push(row);
 }
 return {identity:{fence:initial,ids},same_dom_nodes:true,samples:result};
}
export function reduceCuaMedia(observation,decoder){
 assert.equal(observation.same_dom_nodes,true);assert.ok(observation.identity?.fence&&observation.identity?.ids);
 assert.ok(Array.isArray(observation.samples)&&observation.samples.length===12);
 const samples=observation.samples.map(sample=>({canvas:decodeMediaJpeg(sample.canvas,decoder),video:decodeMediaJpeg(sample.video,decoder),unsupported:decodeMediaJpeg(sample.unsupported,decoder),playback:sample.playback}));
 return {identity:observation.identity,samples};
}
