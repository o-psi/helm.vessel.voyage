const $=id=>document.getElementById(id);
const controller_id=crypto.randomUUID();
let csrf,state,frameURL,stopped=false,composition=false,localGeneration=0;
const secret=location.hash.slice(1);history.replaceState(null,'','/');
const fail=e=>{$('error').textContent=e.message||String(e);};
async function post(endpoint,payload,binary=false){
  const r=await fetch(endpoint,{method:'POST',credentials:'same-origin',cache:'no-store',headers:{'Content-Type':'application/json',...(csrf?{'X-Helm-CSRF':csrf}:{})},body:JSON.stringify(payload)});
  if(!r.ok){const v=await r.json();throw Error(v.error?.code||'Local request refused');}
  return binary?r.blob():r.json();
}
const api=async b=>post('/api',{controller_id,epoch:state?.epoch,...b});
const button=(text,fn)=>{const e=document.createElement('button');e.textContent=text;e.onclick=()=>Promise.resolve().then(fn).catch(fail);return e;};
const text=(parent,value)=>{const p=document.createElement('p');p.textContent=value;parent.append(p);};
function interactive(){return state&&['private','human'].includes(state.mode);}
async function takeover(mode){localGeneration++;state={...state,mode:'private'};await api({op:'mode',mode});await refresh();}
$('private').onclick=()=>takeover('private').catch(fail);$('human').onclick=()=>takeover('human').catch(fail);
$('share').onclick=async()=>{
  if(!confirm('Share this entire browser with the selected Voyage? Visible pages, signed-in data, DOM observations and screenshots can enter remote model/history. Do not share private pages. Existing upload/download grants need separate review.'))return;
  localGeneration++;try{await api({op:'mode',mode:'agent',confirm_share:true});await refresh();}catch(e){fail(e);}
};
$('claim').onclick=()=>api({op:'claim',controller_id}).then(refresh).catch(fail);
$('go').onclick=()=>api({op:'navigate',url:$('url').value}).catch(fail);$('url').onkeydown=e=>{if(e.key==='Enter')$('go').click();};
$('new').onclick=()=>api({op:'new_tab'}).catch(fail);$('close').onclick=()=>api({op:'close_tab'}).catch(fail);
$('allow-origin').onclick=async()=>{try{const url=new URL($('origin').value).origin,private_network=$('private-network').checked;if(!confirm(`Allow ${url}${private_network?' INCLUDING private/LAN/loopback network destinations':''}? This also permits requests from other granted pages to this origin.`))return;await api({op:'origin',url,private_network});await refresh();}catch(e){fail(e);}};
$('paste').onclick=async()=>{try{if(!interactive())throw Error('Take local control before pasting');if(!confirm('Allow this browser page to receive your local clipboard text?'))return;const text=await navigator.clipboard.readText();await api({op:'clipboard',allow:true,text});}catch(e){fail(e);}};
$('upload').onchange=async()=>{try{const f=$('upload').files[0];if(!f)return;if(f.size>2097152)throw Error('Upload limit is 2 MiB');const bytes=new Uint8Array(await f.arrayBuffer());let s='';for(let i=0;i<bytes.length;i+=8192)s+=String.fromCharCode(...bytes.subarray(i,i+8192));await api({op:'upload',name:f.name,mime_type:f.type||'application/octet-stream',data_base64:btoa(s),chooser_id:state.chooser||null});await refresh();}catch(e){fail(e);}finally{$('upload').value='';}};
let rendered='';
async function refresh(){
  const next=await api({op:'state'});if(state?.epoch!==next.epoch)localGeneration++;state=next;
  $('mode').textContent=`${state.mode.toUpperCase()} · epoch ${state.epoch}${state.connected?'':' · Helm disconnected'}`;
  $('label').textContent=[state.label,state.binding?`Voyage ${state.binding.session_id} · run ${state.binding.run_id||'idle'}`:'Unbound local helper'].filter(Boolean).join(' · ');$('viewport').style.aspectRatio=`${state.width} / ${state.height}`;
  for(const id of ['go','url','new','close','allow-origin','upload','paste'])$(id).disabled=!interactive();
  $('share').disabled=state.mode==='agent'||!state.connected;
  const key=JSON.stringify([state.mode,state.epoch,state.tabs,state.selected,state.origins,state.prompts,state.uploads,state.downloads,state.dialog]);
  if(key===rendered)return;rendered=key;
  $('tabs').replaceChildren();for(const t of state.tabs){const b=button(t.title||t.url||'Blank tab',()=>api({op:'tab',page_id:t.id}));if(t.id===state.selected)b.className='selected';b.disabled=!interactive();$('tabs').append(b);}
  if(document.activeElement!==$('url'))$('url').value=state.tabs.find(t=>t.id===state.selected)?.url||'';
  $('origins').replaceChildren();for(const o of state.origins){const div=document.createElement('div');text(div,o.url+(o.private_network?' · PRIVATE NETWORK PERMITTED':''));div.append(button('Revoke',()=>api({op:'origin',url:o.url,remove:true})));$('origins').append(div);}
  $('approval').replaceChildren();for(const p of state.prompts){const div=document.createElement('div'),s=p.summary;text(div,`Voyage requests ${s.type}. Current origin: ${s.current_origin}. Destination: ${s.destination_origin}.`);if(s.url)text(div,`URL: ${s.url}`);if(s.element)text(div,`Page-supplied element description (untrusted): ${JSON.stringify(s.element)}`);if(s.text!==undefined)text(div,`Fill value: ${s.text}`);if(s.filename!==undefined)text(div,`File: ${s.filename} · ${s.bytes} bytes · ${s.mime_type}`);text(div,'Allow this one effect locally? Page content cannot approve permissions.');div.append(button('Allow once',()=>api({op:'confirm',prompt_id:p.id,allow:true})),button('Deny',()=>api({op:'confirm',prompt_id:p.id,allow:false})));$('approval').append(div);}
  $('transfers').replaceChildren();for(const u of state.uploads){const div=document.createElement('div');text(div,`Upload: ${u.name} (${u.size} bytes)${u.shared?' · shared for current epoch':''}`);div.append(button('Grant to Voyage this epoch',async()=>{if(confirm('Disclose this file to the agent-controlled browser? The upload effect will still require local approval.'))await api({op:'share_upload',upload_id:u.id,allow:true});}));div.append(button('Discard locally (take control first)',()=>api({op:'discard_upload',transfer_id:u.id})));$('transfers').append(div);}
  for(const d of state.downloads){const div=document.createElement('div');text(div,`Download: ${d.name} (${d.size} bytes)${d.pending?' · receiving':''}`);if(!d.pending)div.append(button('Save locally',async()=>{const blob=await post('/save',{controller_id,download_id:d.id},true),url=URL.createObjectURL(blob),a=document.createElement('a');a.href=url;a.download='download.bin';a.click();setTimeout(()=>URL.revokeObjectURL(url),10000);}),button('Disclose to Voyage once',async()=>{if(confirm('Send this file to remote Voyage/model? Saving locally is separate.'))await api({op:'disclose_download',download_id:d.id,allow:true});}));div.append(button('Discard locally (take control first)',()=>api({op:'discard_download',transfer_id:d.id})));$('transfers').append(div);}
  $('dialog').replaceChildren();if(state.dialog){text($('dialog'),`${state.dialog.type}: ${state.dialog.message}`);const input=document.createElement('input');input.placeholder='Prompt response (local only)';$('dialog').append(input,button('Accept',()=>api({op:'dialog',dialog_id:state.dialog.id,accept:true,text:input.value})),button('Dismiss',()=>api({op:'dialog',dialog_id:state.dialog.id,accept:false})));}
}
// Sequential local input, stamped when received. Takeover discards old queued input.
let inputQueue=Promise.resolve(),inputPending=0;
function sendInput(b){if(!interactive())return;if(inputPending>=128){fail(Error('Local input queue is full; release controls and retry'));return;}inputPending++;const generation=localGeneration,epoch=state.epoch;inputQueue=inputQueue.then(()=>{if(generation!==localGeneration)return;return api({...b,epoch});}).catch(fail).finally(()=>inputPending--);}
const viewport=$('viewport');
function coords(e){const r=viewport.getBoundingClientRect();return{x:Math.max(0,Math.min(state.width,(e.clientX-r.left)*state.width/r.width)),y:Math.max(0,Math.min(state.height,(e.clientY-r.top)*state.height/r.height))};}
viewport.onpointerdown=e=>{if(!interactive())return;e.preventDefault();viewport.focus({preventScroll:true});viewport.setPointerCapture(e.pointerId);sendInput({op:'pointer',kind:'down',button:e.button,click_count:e.detail===2?2:1,...coords(e)});};
viewport.onpointerup=e=>{if(!interactive())return;e.preventDefault();sendInput({op:'pointer',kind:'up',button:e.button,click_count:e.detail===2?2:1,...coords(e)});};
let lastMove=0;viewport.onpointermove=e=>{if(!interactive()||Date.now()-lastMove<30)return;lastMove=Date.now();sendInput({op:'pointer',kind:'move',...coords(e)});};
viewport.oncontextmenu=e=>e.preventDefault();
viewport.addEventListener('wheel',e=>{if(!interactive())return;e.preventDefault();sendInput({op:'wheel',x:Math.max(-5000,Math.min(5000,e.deltaX)),y:Math.max(-5000,Math.min(5000,e.deltaY))});},{passive:false});
function key(e,kind){if(!interactive()||composition||e.isComposing)return;if(e.key==='Unidentified'||e.key==='Dead')return;e.preventDefault();const key=e.key===' '?'Space':e.key;sendInput({op:'key',kind,key});}
viewport.onkeydown=e=>key(e,'down');viewport.onkeyup=e=>key(e,'up');
viewport.addEventListener('compositionstart',()=>composition=true);viewport.addEventListener('compositionend',e=>{composition=false;sendInput({op:'text',text:e.data});$('ime').value='';});
// A focusable textarea allows native IME composition; printable key forwarding uses the same page.
viewport.ondblclick=()=>{if(interactive())$('ime').focus();};
$('ime').addEventListener('beforeinput',e=>{if(e.inputType==='insertFromPaste')e.preventDefault();});
window.addEventListener('blur',()=>{localGeneration++;sendInput({op:'release_input'});});
async function frames(){
  while(!stopped){try{if(state){const epoch=state.epoch;const blob=await post('/frame',{controller_id},true);if(epoch===state.epoch){const old=frameURL;frameURL=URL.createObjectURL(blob);$('frame').src=frameURL;if(old)URL.revokeObjectURL(old);$('frame-status').textContent=`Live local viewport · ${state.width}×${state.height} · no recordings`;}}}catch(e){$('frame-status').textContent=`Frame unavailable: ${e.message}`;}await new Promise(r=>setTimeout(r,100));}
}
try{({csrf}=await post(secret?'/session':'/resume',secret?{secret}:{}));await api({op:'claim',controller_id});await refresh();void frames();setInterval(()=>refresh().catch(fail),500);}catch(e){fail(e);}
window.addEventListener('pagehide',()=>{stopped=true;if(frameURL)URL.revokeObjectURL(frameURL);});
