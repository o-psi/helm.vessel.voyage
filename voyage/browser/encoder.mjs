// Runs exclusively in a separate trusted Chromium process. No task JS, HTTP server,
// external script, arbitrary eval API or task-page RTC capability reaches this page.
export function encoderRuntime() {
  const canvas=document.createElement('canvas');canvas.width=1280;canvas.height=720;
  document.body.replaceChildren(canvas);const context=canvas.getContext('2d');
  const stream=canvas.captureStream(0),track=stream.getVideoTracks()[0],peers=new Map();
  track.contentHint='detail';
  let epoch=0,hasFrame=false;
  // A static page may finish painting before ICE connects. Keep its current
  // pixels available to newly connected receivers without recapturing the page.
  const present=()=>{if(!hasFrame||![...peers.values()].some(p=>p.connectionState==='connected'))return;context.drawImage(canvas,0,0);track.requestFrame();};
  const refresh=setInterval(present,250);
  globalThis.encoder={
    peerCount(){return peers.size;},
    async frame({data,width,height,generation}){
      if(generation!==epoch)return;
      const bytes=Uint8Array.from(atob(data),c=>c.charCodeAt(0));
      const bitmap=await createImageBitmap(new Blob([bytes],{type:'image/jpeg'}));
      try{if(generation!==epoch)return;if(canvas.width!==width||canvas.height!==height){canvas.width=width;canvas.height=height;}context.drawImage(bitmap,0,0,width,height);hasFrame=true;track.requestFrame();}finally{bitmap.close();}
    },
    async reset(generation){epoch=generation;hasFrame=false;for(const p of peers.values())p.close();peers.clear();context.clearRect(0,0,canvas.width,canvas.height);track.requestFrame();},
    async offer({viewer,iceServers,relayOnly}){
      peers.get(viewer)?.close();const pc=new RTCPeerConnection({iceServers,iceTransportPolicy:relayOnly?'relay':'all',bundlePolicy:'max-bundle'});peers.set(viewer,pc);pc.onconnectionstatechange=present;
      const sender=pc.addTrack(track,stream);const params=sender.getParameters();params.encodings=[{maxBitrate:2000000,maxFramerate:30}];params.degradationPreference='maintain-resolution';await sender.setParameters(params);
      const transceiver=pc.getTransceivers()[0];const codecs=RTCRtpSender.getCapabilities('video').codecs.filter(c=>c.mimeType==='video/VP8'||c.mimeType==='video/rtx');transceiver.setCodecPreferences(codecs);
      await pc.setLocalDescription(await pc.createOffer());
      await new Promise((resolve,reject)=>{if(pc.iceGatheringState==='complete')return resolve();const timer=setTimeout(()=>{pc.close();peers.delete(viewer);reject(Error('ice_timeout'));},8000);pc.onicegatheringstatechange=()=>{if(pc.iceGatheringState==='complete'){clearTimeout(timer);resolve();}};});
      return {type:pc.localDescription.type,sdp:pc.localDescription.sdp};
    },
    async answer({viewer,description}){const pc=peers.get(viewer);if(!pc)throw Error('missing_peer');await pc.setRemoteDescription(description);},
    disconnect(viewer){peers.get(viewer)?.close();peers.delete(viewer);},
    stop(){clearInterval(refresh);hasFrame=false;for(const pc of peers.values())pc.close();peers.clear();stream.getTracks().forEach(t=>t.stop());},
  };
}

// At most one evaluating JPEG and one newest pending JPEG; no FIFO media backlog.
export class LatestFrameQueue {
  constructor(deliver){this.deliver=deliver;this.pending=null;this.running=null;this.generation=0;this.accepted=0;this.replaced=0;this.failures=0;}
  push(frame){if(this.pending)this.replaced++;this.pending={...frame,generation:this.generation};this.accepted++;if(!this.running){this.running=this.drain().finally(()=>{this.running=null;});}return this.running;}
  async drain(){while(this.pending){const frame=this.pending;this.pending=null;try{await this.deliver(frame);}catch{this.failures++;}}}
  async fence(generation){this.generation=generation;this.pending=null;await this.running;}
}
