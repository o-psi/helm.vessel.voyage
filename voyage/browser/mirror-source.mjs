// Runs inside the task page. Website scripts may change their own DOM, but they
// never receive viewer authority. The worker reads this bounded recorder through
// its private Playwright pipe; no page network channel is created.
(() => {
  const library = globalThis.rrweb;
  delete globalThis.rrweb;
  const LIMIT = 2500000;
  let stop = null, sequence = 0, size = 0, overflow = false, unavailable = null;
  let events = [], metadata = null;

  function emit(event) {
    if (unavailable || overflow) return;
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
    stop = library.record({
      emit,
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

  globalThis.__voyageMirror = Object.freeze({
    drain(since, budget = 2200000) {
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
    stop() {
      stop?.();stop=null;events=[];size=0;overflow=false;metadata=null;
    },
  });
})();
