import test from 'node:test';
import assert from 'node:assert/strict';
import {Creation,accountChoices} from '../resources/react/settings.ts';
class Storage {data=new Map<string,string>();get length(){return this.data.size;}key(i:number){return [...this.data.keys()][i]??null;}getItem(key:string){return this.data.get(key)??null;}setItem(key:string,value:string){this.data.set(key,value);}removeItem(key:string){this.data.delete(key);}clear(){this.data.clear();}}
test('creation uncertainty uses original identity and never submits a prompt',async()=>{
 const storage=new Storage(),creation=new Creation(storage as any,'tenant'),commands:any[]=[];
 const c:any={id:'connection',vessel_id:'vessel',voyages:[],client:{async exchange({command}:any){commands.push(command);if(command.op==='start_account')throw new Error('lost reply');return {protocol:1,outcome_unknown:false,error:null,result:{command_id:command.command_id,session_id:command.session_id,status:'created',process:{session_id:command.session_id,workspace:command.workspace,incarnation:'incarnation'}}};}}};
 await assert.rejects(()=>creation.start(c,'/workspace',{model:'model',account:{account_id:'account'}}));
 await assert.rejects(()=>creation.start(c,'/workspace',{model:'model'}),/unconfirmed/);
 assert.equal(commands.length,1);const record=creation.pending()[0];const process=await creation.reconcile(c,record);
 assert.equal(process.session_id,commands[0].session_id);assert.equal(commands[1].command_id,commands[0].command_id);assert.equal(commands[1].op,'resolve_start_account');assert.equal(storage.length,0);assert.equal(commands.some(c=>c.op==='submit'),false);
});
test('account choices retain full versioned bindings and disable unavailable accounts',()=>{
 const choices=accountChoices({accounts:[{id:'a',connection_id:'c',identity_generation:4,label:'Work',state:'ready',availability:'available'},{id:'b',connection_id:'c',identity_generation:5,label:'Unavailable',state:'expired',availability:'unavailable'}],connections:[{id:'c',revision:7,label:'Provider',transports:['chatgpt_oauth']}]});
 assert.deepEqual(choices[0].binding,{account_id:'a',connection_id:'c',identity_generation:4,connection_revision:7,transport:'chatgpt_oauth'});assert.equal(choices[0].ready,true);assert.equal(choices[1].ready,false);
});
test('creation accepts server canonical workspace with exact session identity',async()=>{
 const storage=new Storage(),creation=new Creation(storage as any,'t');const connection:any={id:'c',vessel_id:'v',voyages:[],client:{async exchange({command}:any){return {protocol:1,outcome_unknown:false,result:{session_id:command.session_id,workspace:'/canonical/work',incarnation:'i'}};}}};
 const process=await creation.start(connection,'/symlink/work/',{model:'m'});assert.equal(process.workspace,'/canonical/work');assert.equal(storage.length,0);
});
