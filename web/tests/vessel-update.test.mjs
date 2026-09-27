import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {spawnSync} from 'node:child_process';
import {mkdtempSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {vesselUpdate} from '../resources/js/vessel-update.js';
import {voyageSettings} from '../resources/js/voyage-settings.js';

const settle=async()=>{for(let i=0;i<10;i++)await new Promise(r=>setTimeout(r,2));};
function fixture(t) {
 const dir=mkdtempSync(join(tmpdir(),'helm-update-'));
 const p=spawnSync('php',['-r',`require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); echo view('livewire.console',['vessels'=>collect(),'tenantId'=>'update-test'])->render();`],{cwd:new URL('..',import.meta.url),encoding:'utf8',env:{...process.env,VIEW_COMPILED_PATH:dir}});
 assert.equal(p.status,0,p.stderr);
 const dom=new JSDOM(p.stdout,{url:'https://helm.example'});
 t.after(()=>{dom.window.close();rmSync(dir,{recursive:true,force:true});});
 for(const key of ['window','document','localStorage','Event'])globalThis[key]=dom.window[key];
 const root=document.querySelector('#helm-client');
 return {root,$:id=>root.querySelector(`#${id}`)};
}
const reply=result=>({protocol:1,error:null,outcome_unknown:false,result});
test('an old Vessel is capability-gated before profiles, preserving the draft',async t=>{
 const {root,$}=fixture(t),seen=[];
 const c={id:'v',vessel_id:'identity',name:'HelmWeb',client:{exchange:async({command})=>{seen.push(command.op);assert.equal(command.op,'capabilities');return reply({vessel_id:'identity',version:'1.0.1',scope:'owner',features:['provider_accounts'],workspaces:[]});}}};
 let origin;
 voyageSettings(root,{connections:new Map([['v',c]])},{current:()=>({vessel:'v'}),select:()=>{},apply:()=>assert.fail(),draft:async(vessel,workspace)=>{origin={key:'draft',vessel,workspace,target:{type:'new_chat',workspace}};},captureDraft:()=>origin});
 $('new-voyage').click();await settle();
 assert.deepEqual(seen,['capabilities']);
 assert.equal($('setup-update').hidden,false);
 assert.match($('update-status').textContent,/one-time remote administrator/);
 assert.equal($('update-source').hidden,true);
 assert.equal(origin.key,'draft');
});
test('prepare requires a separate exact approval and lost apply reply is only observed',async t=>{
 const {root,$}=fixture(t),seen=[];let record={phase:'idle'},loseApply=false;
 const c={id:'v',vessel_id:'identity',name:'HelmWeb',client:{exchange:async({command})=>{
   seen.push(command);
   if(command.op==='capabilities')return reply({vessel_id:'identity',running_release:'a'.repeat(64),features:['execution_profiles']});
   if(command.op==='update_prepare')record={operation_id:command.operation_id,channel:command.channel,phase:'ready',version:'next',release_id:'a'.repeat(64),description:'pinned source',services:['voyage-vessel.service']};
   if(command.op==='update_apply'){assert.equal(command.release_id,'a'.repeat(64));record={...record,phase:'complete'};if(loseApply)throw Error('Disconnected');}
   return reply(structuredClone(record));
 }}};
 let resumed=0;const controller=vesselUpdate(root,{show:()=>{},resume:()=>{resumed++;}});
 controller.bind(c,{remote_updates:true,scope:'owner',version:'old'});
 $('update-channel').value='nightly';$('update-check').click();await settle();
 assert.equal(seen.filter(c=>c.op==='update_prepare').length,1);
 assert.equal(seen.some(c=>c.op==='update_apply'),false);
 assert.equal($('update-review').hidden,false);
 loseApply=true;$('update-approve').click();$('update-approve').click();await settle();
 assert.equal(seen.filter(c=>c.op==='update_apply').length,1);
 assert.equal($('update-continue').hidden,false);
 controller.bind(c,{remote_updates:true,scope:'owner'});$('setup-update-open').click();await settle();
 assert.equal(seen.filter(c=>c.op==='update_apply').length,1,'reopening observes the receipt without reapplying');
 $('update-continue').click();assert.equal(resumed,1);
});
test('switching Vessel drops stale update replies and owner-only controls stay unavailable',async t=>{
 const {root,$}=fixture(t);let resolve;
 const c={id:'a',vessel_id:'a',name:'A',client:{exchange:()=>new Promise(r=>{resolve=r;})}};
 const update=vesselUpdate(root,{show:()=>{},resume:()=>{}});
 update.bind(c,{remote_updates:true,scope:'owner'});$('update-check').click();
 update.bind({id:'b',vessel_id:'b',name:'B'},{remote_updates:false,scope:'workspaces'});
 resolve(reply({phase:'ready',release_id:'b'.repeat(64),version:'wrong Vessel'}));await settle();
 assert.equal($('update-review').hidden,true);
 assert.match($('update-status').textContent,/account owner/);
 assert.match($('update-vessel-name').textContent,/B/);
});

test('update transport only accepts fixed typed requests and projects owner capability',async()=>{
 const {validCommand,restrictCapabilities}=await import('../gateway/protocol.js');
 const id='10000000-0000-4000-8000-000000000001';
 const frame=command=>({type:'command',request_id:id,request:{protocol:1,command}});
 for(const command of [{op:'update_prepare',operation_id:id,channel:'nightly'},{op:'update_status',operation_id:id},{op:'update_apply',operation_id:id,release_id:'a'.repeat(64)},{op:'update_discard',operation_id:id}]) {
   assert.equal(validCommand(frame(command)),true);
   assert.equal(validCommand(frame({...command,url:'https://untrusted.example/program'})),false);
   assert.equal(validCommand(frame({...command,command:'anything'})),false);
 }
 assert.equal(validCommand(frame({op:'update_prepare',operation_id:id,channel:'arbitrary'})),false);
 assert.equal(validCommand(frame({op:'update_apply',operation_id:id,release_id:'changed'})),false);
 const caps={protocol:1,vessel_id:id,scope:'workspaces',features:[],remote_updates:true,running_release:'a'.repeat(64)};
 assert.equal(restrictCapabilities(caps,id).remote_updates,false);
 assert.equal(restrictCapabilities({...caps,scope:'owner'},id).remote_updates,true);
});
