// Runs inside the task page. Website scripts may change their own DOM, but they
// never receive viewer authority. The worker reads this bounded recorder through
// its private Playwright pipe; no page network channel is created.
(() => {
  'use strict';
  // Repeat initialization keeps the immutable source-created closure. The first
  // init script runs before site scripts; they cannot replace this global slot.
  if(Object.getOwnPropertyDescriptor(globalThis,'__voyageMirror')?.configurable===false){delete globalThis.rrweb;return;}
  const library = globalThis.rrweb;
  delete globalThis.rrweb;
  const CAPABILITY='__VOYAGE_CAPTURE_KEY__';
  let enabled=false,captureGeneration=0;
  const authorized=(key,generation)=>key===CAPABILITY&&Number.isSafeInteger(generation)&&generation>=captureGeneration;
  const LIMIT = 2500000;
  let stop = null, sequence = 0, size = 0, overflow = false, unavailable = null;
  let events = [], metadata = null;
  const cssCodec=typeof __VOYAGE_CSS_CODEC__==='function'?__VOYAGE_CSS_CODEC__():null;
  const Compressor=globalThis.CompressionStream;
  let cssMode=false,cssSize=0,cssLane=Promise.resolve();const pendingCss=new Set();
  const compactEntry=event=>{
    const packed=cssCodec.pack([event]);
    return {event:packed.payload.events[0],css_dictionary:packed.payload.css_dictionary,expanded_bytes:packed.expanded_bytes,retained_bytes:cssCodec.jsonBytes(packed.payload)};
  };
  const expanded=entry=>cssMode?cssCodec.unpack({events:[entry.event],css_dictionary:entry.css_dictionary}).events[0]:entry.event;
  async function chunks(payload,generation,token){
    token.bytes=cssCodec.jsonBytes(payload);
    const bytes=new TextEncoder().encode(JSON.stringify(payload));
    if(bytes.length>cssCodec.limit)throw Error('css_transport_limit');
    const parts=[];
    for(let offset=0;offset<bytes.length;offset+=cssCodec.chunkBytes){
      if(!enabled||generation!==captureGeneration)throw Error('capture_fenced');
      const compressed=new Uint8Array(await new Response(new Blob([bytes.subarray(offset,offset+cssCodec.chunkBytes)]).stream().pipeThrough(new Compressor('gzip'))).arrayBuffer());
      if(!enabled||generation!==captureGeneration)throw Error('capture_fenced');
      let text='';for(let at=0;at<compressed.length;at+=8192)text+=String.fromCharCode(...compressed.subarray(at,at+8192));
      parts.push(btoa(text));
    }
    if(parts.length>cssCodec.maxChunks)throw Error('css_transport_limit');
    return {encoding:'gzip-chunks',format:'css_chunks_v1',chunks:parts,total_bytes:bytes.length};
  }

  function emit(event) {
    if (!enabled || unavailable || overflow) return;
    let item={event},length,retained=0;
    try{if(cssMode){item=compactEntry(event);retained=item.retained_bytes;}length=JSON.stringify(item.event).length;}
    catch{unavailable='page_too_large';events=[];size=0;cssSize=0;return;}
    if (length > LIMIT) { unavailable = 'page_too_large'; events = []; size=0;cssSize=0;return; }
    if (size + length > LIMIT || events.length >= 1024 || cssMode&&cssSize+retained>cssCodec.limit) { overflow = true; return; }
    if (event.type === 4) metadata = item;
    const entry = {sequence: ++sequence, ...item};
    events.push(entry); size += length;cssSize+=retained;
  }

  function start() {
    if (stop || unavailable) return;
    if (!library?.record) { unavailable = 'recorder_unavailable'; return; }
    const startedGeneration=captureGeneration;
    stop = library.record({
      emit:event=>{if(enabled&&captureGeneration===startedGeneration)emit(event);},
      inlineStylesheet: true,
      inlineImages: true,
      collectFonts: true,
      maskInputOptions: {password: true},
      sampling: {mousemove: false, mouseInteraction: false, scroll: 100},
    });
    if (!stop) unavailable = 'recorder_unavailable';
  }

  function checkout() {
    if (!stop || unavailable) return;
    events = []; size = 0; cssSize=0; overflow = false;
    library.record.takeFullSnapshot();
    if (!events.some(({event}) => event.type === 2)) unavailable = 'snapshot_unavailable';
  }

  // Keep the source-created recorder closure authoritative after site scripts
  // begin. Neither its object nor global slot can be replaced with a claimed ack.
  Object.defineProperty(globalThis,'__voyageMirror',{writable:false,configurable:false,value:Object.freeze({
    enable(key,generation){
      if(!authorized(key,generation))return false;
      if(generation>captureGeneration){stop?.();stop=null;events=[];size=0;cssSize=0;overflow=false;metadata=null;}
      captureGeneration=generation;enabled=true;return true;
    },
    drain(since, budget = 2200000,key,generation) {
      if(!enabled||key!==CAPABILITY||generation!==captureGeneration)return {error:'recorder_disabled'};
      if (!Number.isSafeInteger(since) || since < 0) return {error:'invalid_cursor'};
      start();
      if (unavailable) return {error:unavailable};
      const first = events[0]?.sequence ?? sequence + 1;
      if (!events.some(({event}) => event.type === 2) || overflow || (since && since < first - 1) || since > sequence) checkout();
      if (unavailable) return {error:unavailable};
      let reset = since === 0 || since < events[0].sequence - 1;
      let candidates;
      if (reset) {
        const index = events.findLastIndex(({event}) => event.type === 2);
        candidates = metadata ? [{sequence:events[index].sequence - 1,...metadata}, ...events.slice(index)] : events.slice(index);
      } else candidates = events.filter(entry => entry.sequence > since);
      const batch = []; let used = 0, cursor = since;
      for (const entry of candidates) {
        const event=expanded(entry);
        const length = JSON.stringify(event).length;
        if(length>LIMIT)return {error:'page_too_large'};
        if (used + length > budget) {
          if (!batch.length) return {error:'event_too_large'};
          break;
        }
        batch.push(event);used += length;cursor = Math.max(cursor,entry.sequence);
      }
      return {events:batch,cursor,reset,latest:sequence};
    },
    async drainCss(since,budget=2200000,key,generation){
      if(!enabled||key!==CAPABILITY||generation!==captureGeneration)return {error:'recorder_disabled'};
      if(!cssCodec||typeof Compressor!=='function')return {error:'unsupported_format'};
      if(!Number.isSafeInteger(since)||since<0)return {error:'invalid_cursor'};
      if(pendingCss.size>=4)return {error:'capture_busy'};
      const token={bytes:0};pendingCss.add(token);let release;const preceding=cssLane;cssLane=new Promise(resolve=>{release=resolve;});
      try{
      await preceding;
      if(!enabled||key!==CAPABILITY||generation!==captureGeneration)return {error:'capture_fenced'};
      if(!cssMode){stop?.();stop=null;events=[];size=0;cssSize=0;overflow=false;metadata=null;unavailable=null;cssMode=true;}
      start();if(unavailable)return {error:unavailable};
      const first=events[0]?.sequence??sequence+1;
      if(!events.some(({event})=>event.type===2)||overflow||(since&&since<first-1)||since>sequence)checkout();
      if(unavailable)return {error:unavailable};
      const reset=since===0||since<events[0].sequence-1;
      const index=events.findLastIndex(({event})=>event.type===2);
      const candidates=reset?(metadata?[{sequence:events[index].sequence-1,...metadata},...events.slice(index)]:events.slice(index)):events.filter(entry=>entry.sequence>since);
      const batch=[];let used=0,cursor=since,logical=2;
      try{
        for(const entry of candidates){
          const length=JSON.stringify(entry.event).length;
          if(used+length>budget||logical+entry.expanded_bytes>cssCodec.limit){if(!batch.length)return {error:'event_too_large'};break;}
          batch.push(expanded(entry));used+=length;logical+=entry.expanded_bytes;cursor=Math.max(cursor,entry.sequence);
        }
        if(reset&&!batch.some(event=>event.type===2))return {error:'event_too_large'};
        const packed=cssCodec.pack(batch),encoded=await chunks(packed.payload,generation,token);
        if(!enabled||key!==CAPABILITY||generation!==captureGeneration)return {error:'capture_fenced'};
        const result={...encoded,cursor,reset,latest:sequence};
        if(JSON.stringify(result).length>budget)return {error:'mirror_limit'};
        return result;
      }catch{return {error:!enabled||generation!==captureGeneration?'capture_fenced':'mirror_limit'};}
      }finally{pendingCss.delete(token);release();}
    },
    node(id) {
      return Number.isSafeInteger(id) && id > 0 ? library?.record?.mirror?.getNode(id) ?? null : null;
    },
    id(node) { return library?.record?.mirror?.getId(node) ?? -1; },
    stop(key,generation) {
      if(!authorized(key,generation))return {error:'stale_capture_permit'};
      captureGeneration=generation;enabled=false;
      stop?.();stop=null;events=[];size=0;cssSize=0;overflow=false;metadata=null;
      return {recording:stop!==null,pending_events:events.length+pendingCss.size,pending_bytes:size+[...pendingCss].reduce((bytes,token)=>bytes+token.bytes,0)};
    },
  })});
})();
