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

  function emit(event) {
    if (!enabled || unavailable || overflow) return;
    const length = JSON.stringify(event).length;
    if (length > LIMIT) { unavailable = 'page_too_large'; events = []; return; }
    if (size + length > LIMIT || events.length >= 1024) { overflow = true; return; }
    if (event.type === 4) metadata = event;
    const entry = {sequence: ++sequence, event};
    events.push(entry); size += length;
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
    events = []; size = 0; overflow = false;
    library.record.takeFullSnapshot();
    if (!events.some(({event}) => event.type === 2)) unavailable = 'snapshot_unavailable';
  }

  // Keep the source-created recorder closure authoritative after site scripts
  // begin. Neither its object nor global slot can be replaced with a claimed ack.
  Object.defineProperty(globalThis,'__voyageMirror',{writable:false,configurable:false,value:Object.freeze({
    enable(key,generation){
      if(!authorized(key,generation))return false;
      if(generation>captureGeneration){stop?.();stop=null;events=[];size=0;overflow=false;metadata=null;}
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
        candidates = metadata ? [{sequence:events[index].sequence - 1,event:metadata}, ...events.slice(index)] : events.slice(index);
      } else candidates = events.filter(entry => entry.sequence > since);
      const batch = []; let used = 0, cursor = since;
      for (const entry of candidates) {
        const length = JSON.stringify(entry.event).length;
        if (used + length > budget) {
          if (!batch.length) return {error:'event_too_large'};
          break;
        }
        batch.push(entry.event);used += length;cursor = Math.max(cursor,entry.sequence);
      }
      return {events:batch,cursor,reset,latest:sequence};
    },
    node(id) {
      return Number.isSafeInteger(id) && id > 0 ? library?.record?.mirror?.getNode(id) ?? null : null;
    },
    id(node) { return library?.record?.mirror?.getId(node) ?? -1; },
    stop(key,generation) {
      if(!authorized(key,generation))return {error:'stale_capture_permit'};
      captureGeneration=generation;enabled=false;
      stop?.();stop=null;events=[];size=0;overflow=false;metadata=null;
      return {recording:stop!==null,pending_events:events.length,pending_bytes:size};
    },
  })});
})();
