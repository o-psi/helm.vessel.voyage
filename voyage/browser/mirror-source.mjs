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
  let cssMode=false,cssSize=0,cssLane=Promise.resolve();const pendingCss=new Set();let encodedBatch=null;const cssJobs=new Map();let jobSequence=0;
  const compactEntry=event=>{
    const packed=cssCodec.pack([event]);
    return {event:packed.payload.events[0],css_dictionary:packed.payload.css_dictionary,expanded_bytes:packed.expanded_bytes,retained_bytes:cssCodec.jsonBytes(packed.payload)};
  };
  const expanded=entry=>cssMode?cssCodec.unpack({events:[entry.event],css_dictionary:entry.css_dictionary}).events[0]:entry.event;
  async function chunks(payload,generation,token,compactBytes){
    token.bytes=compactBytes;
    const bytes=new TextEncoder().encode(JSON.stringify(payload));
    if(bytes.length>cssCodec.limit)throw Error('css_transport_limit');
    const parts=[];
    for(let offset=0;offset<bytes.length;offset+=cssCodec.chunkBytes){
      if(token.cancelled||!enabled||generation!==captureGeneration)throw Error('capture_fenced');
      const compressed=new Uint8Array(await new Response(new Blob([bytes.subarray(offset,offset+cssCodec.chunkBytes)]).stream().pipeThrough(new Compressor('gzip'))).arrayBuffer());
      if(token.cancelled||!enabled||generation!==captureGeneration)throw Error('capture_fenced');
      let text='';for(let at=0;at<compressed.length;at+=8192)text+=String.fromCharCode(...compressed.subarray(at,at+8192));
      parts.push(btoa(text));
    }
    if(parts.length>cssCodec.maxChunks)throw Error('css_transport_limit');
    return {encoding:'gzip-chunks',format:'css_chunks_v1',chunks:parts,total_bytes:bytes.length};
  }

  function cancelJob(job){
    job.token.cancelled=true;if(job.value&&encodedBatch?.value===job.value)encodedBatch=null;job.value=null;
    if(!job.started){clearTimeout(job.timer);pendingCss.delete(job.token);cssJobs.delete(job.id);job.resolve();}
    else if(!pendingCss.has(job.token))cssJobs.delete(job.id);
  }
  function retireJobs(){for(const job of cssJobs.values())cancelJob(job);}

  function emit(event) {
    if (!enabled || unavailable || overflow) return;
    let item={event},length,retained=0;
    try{if(cssMode){item=compactEntry(event);retained=item.retained_bytes;}length=JSON.stringify(item.event).length;}
    catch{unavailable='page_too_large';events=[];size=0;cssSize=0;return;}
    if (length > LIMIT) { unavailable = 'page_too_large'; events = []; size=0;cssSize=0;return; }
    if (size + length > LIMIT || events.length >= 1024 || cssMode&&cssSize+retained>cssCodec.limit) { overflow = true; return; }
    if (event.type === 4) metadata = item;
    encodedBatch=null;
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
    events = []; size = 0; cssSize=0; overflow = false;encodedBatch=null;
    library.record.takeFullSnapshot();
    if (!events.some(({event}) => event.type === 2)) unavailable = 'snapshot_unavailable';
  }

  // Keep the source-created recorder closure authoritative after site scripts
  // begin. Neither its object nor global slot can be replaced with a claimed ack.
  Object.defineProperty(globalThis,'__voyageMirror',{writable:false,configurable:false,value:Object.freeze({
    enable(key,generation){
      if(!authorized(key,generation))return false;
      if(generation>captureGeneration){retireJobs();stop?.();stop=null;events=[];size=0;cssSize=0;overflow=false;metadata=null;encodedBatch=null;}
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
    async drainCss(since,budget=2200000,key,generation,ownedToken=null){
      if(!enabled||key!==CAPABILITY||generation!==captureGeneration)return {error:'recorder_disabled'};
      if(!cssCodec||typeof Compressor!=='function')return {error:'unsupported_format'};
      if(!Number.isSafeInteger(since)||since<0)return {error:'invalid_cursor'};
      if(ownedToken&&!pendingCss.has(ownedToken))return {error:'capture_fenced'};
      if(!ownedToken&&pendingCss.size>=4)return {error:'capture_busy'};
      const token=ownedToken||{bytes:0,cancelled:false};pendingCss.add(token);let release;const preceding=cssLane;cssLane=new Promise(resolve=>{release=resolve;});
      try{
      await preceding;
      if(token.cancelled||!enabled||key!==CAPABILITY||generation!==captureGeneration)return {error:'capture_fenced'};
      if(!cssMode){stop?.();stop=null;events=[];size=0;cssSize=0;overflow=false;metadata=null;encodedBatch=null;unavailable=null;cssMode=true;}
      start();if(unavailable)return {error:unavailable};
      const first=events[0]?.sequence??sequence+1;
      if(!events.some(({event})=>event.type===2)||overflow||(since&&since<first-1)||since>sequence)checkout();
      if(unavailable)return {error:unavailable};
      const reset=since===0||since<events[0].sequence-1;
      const index=events.findLastIndex(({event})=>event.type===2);
      const candidates=reset?(metadata?[{sequence:events[index].sequence-1,...metadata},...events.slice(index)]:events.slice(index)):events.filter(entry=>entry.sequence>since);
      const cacheKey=JSON.stringify([generation,since,budget,sequence]);
      if(encodedBatch?.key===cacheKey)return encodedBatch.value;
      const batch=[];let used=0,cursor=since,logical=2;
      try{
        for(const entry of candidates){
          const length=JSON.stringify(entry.event).length;
          if(used+length>budget||logical+entry.expanded_bytes-2+(batch.length?1:0)>cssCodec.limit){if(!batch.length)return {error:'event_too_large'};break;}
          logical+=entry.expanded_bytes-2+(batch.length?1:0);batch.push(entry);used+=length;cursor=Math.max(cursor,entry.sequence);
        }
        if(reset&&!batch.some(entry=>entry.event.type===2))return {error:'event_too_large'};
        const latest=sequence;
        const packed=cssCodec.combine(batch.map(entry=>({events:[entry.event],css_dictionary:entry.css_dictionary,expanded_bytes:entry.expanded_bytes})));
        const encoded=await chunks(packed.payload,generation,token,packed.compact_bytes);
        if(token.cancelled||!enabled||key!==CAPABILITY||generation!==captureGeneration)return {error:'capture_fenced'};
        const result={...encoded,cursor,reset,latest};
        if(JSON.stringify(result).length>budget)return {error:'mirror_limit'};
        Object.freeze(result.chunks);Object.freeze(result);
        if(sequence===latest&&enabled&&generation===captureGeneration)encodedBatch={key:cacheKey,value:result};
        return result;
      }catch{return {error:token.cancelled||!enabled||generation!==captureGeneration?'capture_fenced':'mirror_limit'};}
      }finally{if(!ownedToken)pendingCss.delete(token);release();}
    },
    beginCss(since,budget,key,generation){
      if(!enabled||key!==CAPABILITY||generation!==captureGeneration)return {error:'capture_fenced'};
      if(!Number.isSafeInteger(since)||since<0||!Number.isSafeInteger(budget)||budget<0||budget>2200000)return {error:'invalid_cursor'};
      const cacheKey=JSON.stringify([generation,since,budget,sequence]);
      if(cssMode&&encodedBatch?.key===cacheKey)return {value:encodedBatch.value,retired:true};
      if(pendingCss.size>=4||cssJobs.size>=4||jobSequence===Number.MAX_SAFE_INTEGER)return {error:'capture_busy'};
      const id=++jobSequence,token={bytes:budget,cancelled:false};
      const job={id,since,budget,generation,token,started:false,value:null,error:null,resolve:null,timer:null};
      job.retirement=new Promise(resolve=>{job.resolve=resolve;});
      pendingCss.add(token);cssJobs.set(id,job);
      job.timer=setTimeout(()=>{
        job.started=true;
        // The registered token owns all work before the first capture or await.
        const work=globalThis.__voyageMirror.drainCss(since,budget,key,generation,token);
        void work.then(value=>{
          if(token.cancelled||!enabled||generation!==captureGeneration)job.error='capture_fenced';
          else if(value?.error)job.error=value.error;
          else job.value=value;
        },()=>{job.error='mirror_limit';}).finally(()=>{
          pendingCss.delete(token);
          if(token.cancelled||generation!==captureGeneration)cssJobs.delete(id);
          job.resolve();
        });
      },0);
      return {job:id,pending:true};
    },
    pollCss(id,since,budget,key,generation){
      if(!enabled||key!==CAPABILITY||generation!==captureGeneration)return {error:'capture_fenced'};
      const job=cssJobs.get(id);
      if(!job||job.generation!==generation||job.since!==since||job.budget!==budget)return {error:'capture_fenced'};
      if(pendingCss.has(job.token))return {job:id,pending:true};
      cssJobs.delete(id);
      return job.error?{job:id,error:job.error,retired:true}:{job:id,value:job.value,retired:true};
    },
    cancelCss(id,key,generation){
      if(key!==CAPABILITY||!Number.isSafeInteger(generation)||generation<0)return {error:'capture_fenced'};
      const job=cssJobs.get(id);if(!job)return {retired:true,pending:false};
      if(generation<job.generation)return {error:'capture_fenced'};
      cancelJob(job);
      return {pending:pendingCss.has(job.token),retired:!pendingCss.has(job.token)};
    },
    node(id) {
      return Number.isSafeInteger(id) && id > 0 ? library?.record?.mirror?.getNode(id) ?? null : null;
    },
    id(node) { return library?.record?.mirror?.getId(node) ?? -1; },
    stop(key,generation) {
      if(!authorized(key,generation))return {error:'stale_capture_permit'};
      captureGeneration=generation;enabled=false;retireJobs();
      stop?.();stop=null;events=[];size=0;cssSize=0;overflow=false;metadata=null;encodedBatch=null;
      return {recording:stop!==null,pending_events:events.length+pendingCss.size,pending_bytes:size+[...pendingCss].reduce((bytes,token)=>bytes+token.bytes,0)};
    },
  })});
})();
