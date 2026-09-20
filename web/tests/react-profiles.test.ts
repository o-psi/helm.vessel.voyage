import test from 'node:test';
import assert from 'node:assert/strict';
import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {JSDOM} from 'jsdom';
import {Settings} from '../resources/react/Settings';
test('profiles editor saves, duplicates, defaults and deletes without mutating voyage settings',async()=>{
 const dom=new JSDOM('<div id="root"></div>',{url:'https://helm.test'});Object.assign(globalThis,{window:dom.window,document:dom.window.document,localStorage:dom.window.localStorage,IS_REACT_ACT_ENVIRONMENT:true});
 dom.window.HTMLDialogElement.prototype.showModal=function(){};dom.window.HTMLDialogElement.prototype.close=function(){};
 const binding={account_id:'a',connection_id:'p',identity_generation:1,connection_revision:1,transport:'openai_responses'};
 let catalogue:any={revision:1,can_manage:true,default_profile_id:'first',profiles:[{id:'first',name:'Everyday',account:binding,model:'m',reasoning_effort:'high',service_tier:null}]};const commands:any[]=[];
 const connection:any={id:'c',name:'Vessel',vessel_id:'v',voyages:[],client:{async exchange({command}:any){commands.push(command);let result:any;
 switch(command.op){case 'capabilities':result={vessel_id:'v',scope:'owner',workspaces:[{path:'/work',name:'Work'}]};break;case 'accounts':result={accounts:[{id:'a',connection_id:'p',identity_generation:1,label:'Work',state:'ready',availability:'available'}],connections:[{id:'p',revision:1,label:'Provider',transports:['openai_responses']}]};break;case 'profiles':result=structuredClone(catalogue);break;case 'account_models':result={account:binding,models:[{id:'m',is_default:true,reasoning_efforts:['high'],service_tiers:[]}]};break;case 'save_profile':assert.equal(command.expected_revision,catalogue.revision);catalogue.profiles=[...catalogue.profiles.filter((p:any)=>p.id!==command.profile.id),command.profile];catalogue.revision++;result=structuredClone(catalogue);break;case 'set_default_profile':catalogue.default_profile_id=command.profile_id;catalogue.revision++;result=structuredClone(catalogue);break;case 'delete_profile':catalogue.profiles=catalogue.profiles.filter((p:any)=>p.id!==command.profile_id);catalogue.revision++;result=structuredClone(catalogue);break;default:throw new Error(command.op);}return {protocol:1,outcome_unknown:false,error:null,result};}}};
 const root=createRoot(document.getElementById('root')!);const click=async(label:string)=>{const button=[...document.querySelectorAll('button')].find(b=>b.textContent===label)!;assert.ok(button,label);await act(async()=>{button.click();});};
 try{await act(async()=>root.render(React.createElement(Settings,{fleet:{connections:new Map([['c',connection]])},workspace:{} as any,tenant:'test',onClose(){},onCreated(){}})));
 assert.equal([...document.querySelectorAll('label')].some(label=>label.textContent?.startsWith('Model')),false);
 assert.equal(commands.filter(command=>command.op==='account_models').length,0,'choosing a saved profile does not discover models');
 await click('Duplicate');assert.ok([...document.querySelectorAll('label')].some(label=>label.textContent?.startsWith('Model')));await click('Save profile');assert.equal(catalogue.profiles.length,2);assert.equal(catalogue.profiles[1].name,'Everyday copy');
 await click('Make default');assert.equal(catalogue.default_profile_id,catalogue.profiles[1].id);await click('Delete');assert.equal(catalogue.profiles.length,1);assert.equal(commands.some(c=>c.op==='set_account_inference'||c.op==='start_account'),false);
 }finally{await act(async()=>root.unmount());dom.window.close();}
});
