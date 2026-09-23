// Trusted encoder page. Task Chromium never receives these RTCPeerConnections.
// The capture track is replaced when the browser viewport changes so receiver
// dimensions follow responsive layouts without rebuilding the media peer.
export function encoderRuntime() {
  let canvas=document.createElement('canvas');canvas.width=1280;canvas.height=720;
  let context=canvas.getContext('2d');document.body.replaceChildren(canvas);
  let stream=canvas.captureStream(0),track=stream.getVideoTracks()[0];track.contentHint='detail';
  const peers=new Map();let epoch=0,hasFrame=false;
  const present=()=>{
    if(!hasFrame||![...peers.values()].some(({pc})=>pc.connectionState==='connected'))return;
    context.drawImage(canvas,0,0);track.requestFrame();
  };
  const refresh=setInterval(present,250);
  const replaceCanvas=async(width,height)=>{
    const next=document.createElement('canvas');next.width=width;next.height=height;
    const nextContext=next.getContext('2d');const nextStream=next.captureStream(0);
    const nextTrack=nextStream.getVideoTracks()[0];nextTrack.contentHint='detail';
    document.body.replaceChildren(next);
    const peerEntries=[...peers];
    const replacements=await Promise.allSettled(peerEntries.map(async([viewer,{sender}])=>{
      await sender.replaceTrack(nextTrack);return viewer;
    }));
    for(let index=0;index<replacements.length;index++)if(replacements[index].status==='rejected'){
      const [viewer,{pc}]=peerEntries[index];pc.close();peers.delete(viewer);
    }
    track.stop();canvas=next;context=nextContext;stream=nextStream;track=nextTrack;
  };
  globalThis.encoder={
    peerCount(){return peers.size;},
    dimensions(){return {width:canvas.width,height:canvas.height};},
    async frame({data,width,height,generation}){
      if(generation!==epoch||!Number.isSafeInteger(width)||!Number.isSafeInteger(height)||width<1||height<1||width>3840||height>2160)return;
      const bytes=Uint8Array.from(atob(data),character=>character.charCodeAt(0));
      const bitmap=await createImageBitmap(new Blob([bytes],{type:'image/jpeg'}));
      try{
        if(generation!==epoch)return;
        if(canvas.width!==width||canvas.height!==height)await replaceCanvas(width,height);
        if(generation!==epoch)return;
        context.drawImage(bitmap,0,0,width,height);hasFrame=true;track.requestFrame();
      }finally{bitmap.close();}
    },
    async fence({generation,keepViewers=[]}){
      epoch=generation;hasFrame=false;
      const keep=new Set(keepViewers);
      for(const [viewer,{pc}] of peers)if(!keep.has(viewer)){pc.close();peers.delete(viewer);}
      context.clearRect(0,0,canvas.width,canvas.height);track.requestFrame();
    },
    async offer({viewer,iceServers,relayOnly}){
      peers.get(viewer)?.pc.close();
      const pc=new RTCPeerConnection({iceServers,iceTransportPolicy:relayOnly?'relay':'all',bundlePolicy:'max-bundle'});
      const sender=pc.addTrack(track,stream);peers.set(viewer,{pc,sender});pc.onconnectionstatechange=present;
      const params=sender.getParameters();params.encodings=[{maxBitrate:2000000,maxFramerate:30}];
      params.degradationPreference='maintain-resolution';await sender.setParameters(params);
      const transceiver=pc.getTransceivers()[0];
      const codecs=RTCRtpSender.getCapabilities('video').codecs.filter(codec=>codec.mimeType==='video/VP8'||codec.mimeType==='video/rtx');
      transceiver.setCodecPreferences(codecs);
      await pc.setLocalDescription(await pc.createOffer());
      await new Promise((resolve,reject)=>{
        if(pc.iceGatheringState==='complete')return resolve();
        const timer=setTimeout(()=>{pc.close();peers.delete(viewer);reject(Error('ice_timeout'));},8000);
        pc.onicegatheringstatechange=()=>{if(pc.iceGatheringState==='complete'){clearTimeout(timer);resolve();}};
      });
      return {type:pc.localDescription.type,sdp:pc.localDescription.sdp};
    },
    async answer({viewer,description}){
      const pc=peers.get(viewer)?.pc;if(!pc)throw Error('missing_peer');await pc.setRemoteDescription(description);
    },
    disconnect(viewer){peers.get(viewer)?.pc.close();peers.delete(viewer);},
    stop(){clearInterval(refresh);hasFrame=false;for(const {pc} of peers.values())pc.close();peers.clear();track.stop();},
  };
}

// One delivering frame and one newest pending frame. Push returns a completion
// promise, allowing CDP screencast acknowledgement after delivery/backpressure.
export class LatestFrameQueue {
  constructor(deliver){this.deliver=deliver;this.pending=null;this.running=null;this.generation=0;this.accepted=0;this.replaced=0;this.failures=0;}
  push(frame){
    if(this.pending)this.replaced++;
    this.pending={...frame,generation:this.generation};this.accepted++;
    if(!this.running)this.running=this.drain().finally(()=>{this.running=null;});
    return this.running;
  }
  async drain(){while(this.pending){const frame=this.pending;this.pending=null;try{await this.deliver(frame);}catch{this.failures++;}}}
  async fence(generation){this.generation=generation;this.pending=null;await this.running;}
}
