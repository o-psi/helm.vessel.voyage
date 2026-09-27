import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {vesselUpdate} from '../resources/js/vessel-update.js';
import React from 'react';
import {renderToStaticMarkup} from 'react-dom/server';
import {VesselUpdateMarkup} from '../resources/react/VesselUpdate.tsx';

const settle=async()=>{for(let i=0;i<10;i++)await new Promise(r=>setTimeout(r,2));};
function fixture(t) {
 const dom=new JSDOM('<div id="helm-client" data-tenant-id="update-test">'+renderToStaticMarkup(React.createElement(VesselUpdateMarkup))+'</div>',{url:'https://helm.example'});
 t.after(()=>dom.window.close());
 for(const key of ['window','document','localStorage','Event'])globalThis[key]=dom.window[key];
 const root=document.querySelector('#helm-client');
 return {root,$:id=>root.querySelector(`#${id}`)};
}
const reply=result=>({protocol:1,error:null,outcome_unknown:false,result});
test('prepare requires a separate exact approval and lost apply reply is only observed',async t=>{
 const {root,$}=fixture(t),seen=[];let record={phase:'idle'},loseApply=false;
 const c={id:'v',vessel_id:'identity',name:'HelmWeb',client:{exchange:async({command})=>{
   seen.push(command);
   if(command.op==='capabilities')return reply({vessel_id:'identity',version:'next',running_release:'a'.repeat(64),features:['execution_profiles']});
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
 assert.equal($('update-current').textContent,'Installed Vessel version: next');
 assert.match($('update-status').textContent,/Reconnected to the verified Vessel version/);
 controller.bind(c,{remote_updates:true,scope:'owner'});$('setup-update-open').click();await settle();
 assert.equal(seen.filter(c=>c.op==='update_apply').length,1,'reopening observes the receipt without reapplying');
 $('update-continue').click();assert.equal(resumed,1);
});
test('reload observes the saved operation before allowing another preparation',async t=>{
 const {root,$}=fixture(t),seen=[];let resolve;
 localStorage.setItem('helm-web:update:update-test:v:identity',JSON.stringify({operation_id:'saved-operation'}));
 const c={id:'v',vessel_id:'identity',name:'HelmWeb',client:{exchange:({command})=>{seen.push(command);return new Promise(r=>{resolve=r;});}}};
 const update=vesselUpdate(root,{show:()=>{},resume:()=>{}});
 update.bind(c,{remote_updates:true,scope:'owner',version:'old'});
 assert.equal($('update-source').hidden,true);
 assert.equal($('update-continue').hidden,true);
 $('update-check').click();
 assert.deepEqual(seen,[{op:'update_status',operation_id:'saved-operation'}]);
 resolve(reply({operation_id:'saved-operation',phase:'ready',release_id:'a'.repeat(64),version:'next'}));await settle();
 assert.equal($('update-review').hidden,false);
 assert.equal($('update-source').hidden,true);
 assert.equal(seen.some(command=>command.op==='update_apply'),false);
});
test('a historical completed update does not block a fresh review or falsely verify another release',async t=>{
 const {root,$}=fixture(t),seen=[];
 let record={operation_id:'older-update',phase:'complete',release_id:'a'.repeat(64),version:'older'};
 const c={id:'v',vessel_id:'identity',name:'HelmWeb',client:{exchange:async({command})=>{
   seen.push(command);
   if(command.op==='capabilities')return reply({vessel_id:'identity',version:'current',running_release:'b'.repeat(64),features:['execution_profiles']});
   if(command.op==='update_prepare')record={operation_id:command.operation_id,phase:'ready',release_id:'c'.repeat(64),version:'next'};
   return reply(structuredClone(record));
 }}};
 const update=vesselUpdate(root,{show:()=>{},resume:()=>assert.fail('unverified continuation')});
 update.bind(c,{remote_updates:true,scope:'owner',version:'current'});
 $('setup-update-open').click();await settle();
 assert.equal($('update-source').hidden,false);
 assert.equal($('update-continue').hidden,true);
 assert.match($('update-status').textContent,/does not match its reviewed release/);
 $('update-check').click();await settle();
 assert.equal(seen.filter(command=>command.op==='update_prepare').length,1);
 assert.equal(seen.some(command=>command.op==='update_apply'),false);
 assert.equal($('update-review').hidden,false);
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

test('disposed update controls ignore late replies and cannot dispatch again',async t=>{
 const {root,$}=fixture(t);let resolve:any;const seen:string[]=[];
 const connection={id:'a',vessel_id:'a',name:'A',client:{exchange:({command}:any)=>{seen.push(command.op);return new Promise(r=>{resolve=r;});}}};
 const controller=vesselUpdate(root,{show:()=>{},resume:()=>{}});
 controller.bind(connection,{remote_updates:true,scope:'owner'});$('update-check').click();controller.dispose();
 resolve(reply({phase:'ready',version:'late',release_id:'a'.repeat(64)}));await settle();
 $('update-check').click();$('update-refresh').click();assert.deepEqual(seen,['update_prepare']);assert.equal($('update-review').hidden,true);
});
