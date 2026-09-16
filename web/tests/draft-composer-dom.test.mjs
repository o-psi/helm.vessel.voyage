import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {draftComposer} from '../resources/js/draft-composer.js';
import {voyageSettings} from '../resources/js/voyage-settings.js';
const dom=new JSDOM('',{url:'https://fixture.invalid'});
for(const key of ['window','document','localStorage','Event'])globalThis[key]=dom.window[key];
const rendered=spawnSync('php',['-r',`require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); echo view('livewire.console',['vessels'=>collect(),'tenantId'=>'test'])->render();`],{cwd:new URL('..',import.meta.url),encoding:'utf8'});
assert.equal(rendered.status,0,rendered.stderr);
const pause=()=>new Promise(r=>setTimeout(r,5));
async function until(fn){for(let i=0;i<150;i++){if(fn())return;await pause();}assert.fail('DOM condition timed out');}
const deferred=()=>{let resolve;return {promise:new Promise(r=>resolve=r),resolve:v=>resolve(v)};};
function server(){
 const records=new Map(),receipts=new Map(),requests=[];
 const bytes=Buffer.from('fixture verified image bytes'),attachment={id:'picture',name:'phone.png',media_type:'image/png',byte_size:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')};
 const state={records,requests,attachment,hold:null,lost:false,reject:false};
 state.client={async exchange({command:c}){requests.push(structuredClone(c));let result;
 if(c.op!=='drafts'){
 if(c.op==='capabilities')result={vessel_id:'owner',scope:'owner',workspaces:[{name:'Repo',path:'/repo'}]};
 else if(c.op==='accounts')result={accounts:[{id:'account',connection_id:'provider',identity_generation:1,label:'Fixture',state:'ready',availability:'available'}],connections:[{id:'provider',label:'Fixture',revision:1,transports:['openai_responses']}]};
 else if(c.op==='account_models')result={account:c.account,models:[{id:'fixture',is_default:true}]};
 else if(c.op==='start_account')result={session_id:c.session_id,incarnation:'inc',workspace:c.workspace};
 else throw Error(c.op);
 }else{const o=c.operation;
 if(o.op==='list')result={drafts:[...records.values()]};
 else if(o.op==='get')result=records.get(o.draft_id)||null;
 else if(o.op==='read_image')result={offset:o.offset,data_base64:bytes.subarray(o.offset,o.offset+o.limit).toString('base64')};
 else if(receipts.has(o.command_id))result=receipts.get(o.command_id);
 else if(o.op==='put'){
 if(state.reject || o.expected_revision!==(records.get(o.draft_id)?.revision||0))return {protocol:1,error:'conflict',outcome_unknown:false};
 result={draft_id:o.draft_id,revision:o.expected_revision+1,document:structuredClone(o.document)};records.set(o.draft_id,result);receipts.set(o.command_id,result);
 }else if(o.op==='delete'){if(o.expected_revision!==records.get(o.draft_id)?.revision)return {protocol:1,error:'conflict',outcome_unknown:false};records.delete(o.draft_id);result={deleted:true};receipts.set(o.command_id,result);}
 else if(o.op==='upload_image')result=attachment;
 else if(o.op==='promote')result={parts:records.get(o.draft_id).document.parts};
 else throw Error(o.op);
 if(state.hold && o.op===state.hold.op){const h=state.hold;state.hold=null;h.entered.resolve();await h.wait.promise;}
 if(state.lost && o.op==='put'){state.lost=false;throw Error('lost acknowledgement');}
 }
 return {protocol:1,error:null,outcome_unknown:false,result:structuredClone(result)};
 }};return state;
}
function device(s,tenant){
 const container=document.createElement('div');container.innerHTML=rendered.stdout;const root=container.querySelector('#helm-client');document.body.append(root);root.dataset.tenantId=tenant;
 const $=q=>root.querySelector(q), fleet={connections:new Map([['a',{id:'a',name:'A',vessel_id:'owner',client:s.client,voyages:[]}],['b',{id:'b',name:'B',vessel_id:'other',client:s.client,voyages:[]}]])};
 let now={vessel:'a',session_id:null,running:false};let composer;
 const select=(vessel,session_id,_title,retain)=>{now={vessel,session_id,running:false};$('#prompt').value='';if(!retain)composer.select();};
 composer=draftComposer(root,{fleet,current:()=>now,select,notice:message=>{$('#notice').textContent=message;}});
 voyageSettings(root,fleet,{current:()=>now,select,draft:(...args)=>composer.newChat(...args),created:(...args)=>composer.created(...args),captureDraft:()=>composer.capture()});
 return {root,$,composer,select,get now(){return now;},type(value){$('#prompt').value=value;$('#prompt').dispatchEvent(new Event('input'));},async choose(id){await composer.select();const picker=root.querySelector('select[aria-label="Shared draft"]');assert.ok(picker);picker.value=id;picker.dispatchEvent(new Event('change'));await until(()=>composer.active?.record?.draft_id===id);await pause();},dispose(){root.remove();}};
}
test('rendered Flux DOM: two devices restore new-chat text/pictures, create through settings then send; clear lost ack + reload retains later edits',async()=>{
 localStorage.clear();const s=server(),a=device(s,'desktop'),b=device(s,'phone');
 try{
 await a.composer.newChat('a','/repo');a.type('desktop draft');await pause();await a.composer.active.save();const id=a.composer.active.record.draft_id;
 a.composer.active.edit([...a.composer.active.document.parts,{type:'image',attachment:s.attachment}]);await a.composer.active.save();
 await b.choose(id);assert.equal(b.$('#prompt').value,'desktop draft');await until(()=>b.$('[data-images] img')?.hidden===false);
 b.type('phone reply');await pause();await b.composer.active.save();await a.choose(id);assert.equal(a.$('#prompt').value,'phone reply');assert.equal(a.$('[data-image-name]').textContent,'phone.png');
 a.$('#new-voyage').click();await until(()=>!a.$('#settings-save').disabled);a.$('#voyage-settings-form').dispatchEvent(new Event('submit',{cancelable:true}));await until(()=>a.now.session_id);
 assert.equal(a.$('#prompt').value,'phone reply');const sent=await a.composer.beforeSend();assert.equal(sent.record.draft_id,id);assert.equal((await a.composer.promote(sent,a.now.session_id))[1].attachment.id,'picture');
 // Merely reopening the composer is not execution admission.
 const revision=s.records.get(id).revision;await a.choose(id);assert.equal(a.$('#prompt').value,'phone reply');assert.equal(s.records.get(id).revision,revision);
 s.lost=true;await assert.rejects(a.composer.admitted(sent));const clear=a.composer.active.deletion.command_id;
 a.dispose();const reload=device(s,'desktop');try{await reload.choose(id);reload.type('after lost ack');await pause();await reload.composer.active.save();assert.equal(s.records.get(id).document.parts[0].text,'after lost ack');assert.equal(reload.composer.active.deletion,null);assert.equal(s.requests.filter(c=>c.operation?.command_id===clear).length,2);}finally{reload.dispose();}
 }finally{a.dispose();b.dispose();}
});
test('DOM async save navigation aborts send; upload and fork completion never replace selected composer',async()=>{
 localStorage.clear();const s=server(),a=device(s,'races');try{
 await a.composer.newChat('a','/repo');a.type('origin');await pause();const origin=a.composer.active;
 const entered=deferred(),wait=deferred();s.hold={op:'put',entered,wait};const send=a.composer.beforeSend();await entered.promise;
 a.select('b','other');await pause();a.type('other text');await pause();wait.resolve();await assert.rejects(send,/selection changed/);assert.equal(a.$('#prompt').value,'other text');assert.notEqual(a.composer.active,origin);
 await a.composer.newChat('a','/repo');a.type('upload origin');await pause();const uploadDraft=a.composer.active;
 const h={op:'upload_image',entered:deferred(),wait:deferred()};s.hold=h;
 const event=new Event('paste',{bubbles:true,cancelable:true});Object.defineProperty(event,'clipboardData',{value:{files:[{type:'image/png',size:4,name:'phone.png',arrayBuffer:async()=>new Uint8Array([1,2,3,4]).buffer}]}});a.$('#composer').dispatchEvent(event);await h.entered.promise;
 a.select('b','another');await pause();a.type('new edits');await pause();h.wait.resolve();await until(()=>uploadDraft.document.parts.some(p=>p.type==='image'));assert.equal(a.$('#prompt').value,'new edits');assert.equal(a.composer.active.document.parts.some(p=>p.type==='image'),false);
 await a.composer.newChat('a','/repo');a.type('fork original');await pause();await a.composer.active.save();const forkOrigin=a.composer.active;
 forkOrigin.conflict={...forkOrigin.record};forkOrigin.changed(forkOrigin);
 const f={op:'put',entered:deferred(),wait:deferred()};s.hold=f;a.$('[data-fork]').click();await f.entered.promise;
 a.type('typed during fork');await pause();f.wait.resolve();await until(()=>[...s.records.values()].some(r=>r.draft_id!==forkOrigin.record.draft_id && r.document.parts[0]?.text==='fork original'));
 await pause();assert.equal(a.composer.active,forkOrigin);assert.equal(a.$('#prompt').value,'typed during fork');
 }finally{a.dispose();}
});
test('DOM creation completion is bound to captured composer, not a newly selected new chat',async()=>{
 localStorage.clear();const s=server(),a=device(s,'creation-race');try{
 await a.composer.newChat('a','/repo');a.type('first');await pause();const captured=a.composer.capture();
 a.$('[data-new]').click();await pause();a.type('second');await pause();const second=a.composer.active;
 assert.equal(await a.composer.created('a','created-session','/repo',captured),true);
 assert.equal(a.now.session_id,null);assert.equal(a.composer.active,second);assert.equal(a.$('#prompt').value,'second');
 }finally{a.dispose();}
});

test('DOM explicit shared discard removes only reviewed revision and retains newer remote edit',async()=>{
 localStorage.clear();const s=server(),a=device(s,'discard');const previous=window.confirm;window.confirm=()=>true;
 try{
 await a.composer.newChat('a','/repo');a.type('keep until confirmed');await pause();await a.composer.active.save();
 const draft=a.composer.active,id=draft.record.draft_id;
 s.records.set(id,{...s.records.get(id),revision:draft.record.revision+1});
 a.$('[data-discard]').click();await until(()=>a.$('#notice').textContent.includes('retained'));
 assert.equal(a.$('#prompt').value,'keep until confirmed');assert.ok(s.records.has(id));
 // Explicitly restore the reviewed shared version before requesting its deletion.
 localStorage.removeItem(`${draft.key}:discard`);await draft.open(s.records.get(id));
 a.$('[data-discard]').click();await until(()=>!s.records.has(id) && a.composer.active===null);assert.equal(a.composer.active,null);
 }finally{window.confirm=previous;a.dispose();}
});

test('quiet composer: no permanent draft form, file picker is hidden, contextual pictures and conflicts only',async()=>{
 localStorage.clear();const s=server(),a=device(s,'compact');try{
 const prompt=a.$('#prompt'),context=a.$('[data-draft-context]'),menu=a.$('[data-draft-menu]');
 assert.equal(context.parentElement,prompt);assert.equal(context.hidden,true);
 assert.equal(a.$('#picture-files').hidden,true);assert.ok(prompt.contains(a.$('#attach-picture')));
 assert.ok(menu.hasAttribute('popover'));assert.ok(menu.contains(a.$('[data-discard]')));
 assert.equal(a.$('[data-draft-status]').textContent,'');assert.equal(a.$('#settings-draft'),null);
 let picks=0;a.$('#picture-files').click=()=>picks++;a.$('#attach-picture').click();assert.equal(picks,1);
 await a.composer.newChat('a','/repo');a.type('A quiet draft');await pause();await a.composer.active.save();
 assert.equal(context.hidden,true);assert.equal(a.$('[data-draft-status]').textContent,'Saved');
 const draft=a.composer.active;draft.conflict={...draft.record};draft.changed(draft);
 assert.equal(context.hidden,false);assert.equal(a.$('[data-draft-conflict]').hidden,false);
 assert.equal(a.$('[data-images]').hidden,true);
 draft.conflict=null;draft.edit([{type:'image',attachment:s.attachment}]);
 assert.equal(a.$('[data-draft-conflict]').hidden,true);assert.equal(a.$('[data-images]').hidden,false);
 assert.equal(a.$('[data-images]').querySelectorAll('[data-picture-card]').length,1);
 assert.ok(a.$('[data-picture-card] button svg'));assert.doesNotMatch(a.$('[data-picture-card] button').textContent,/Remove/);
 assert.match(context.className,/col-span-4/);assert.match(a.$('#composer').className,/max-w-3xl/);assert.equal(prompt.getAttribute('rows'),'2');
 a.$('[data-picture-card] button').click();assert.equal(context.hidden,true);
 }finally{a.dispose();}
});
