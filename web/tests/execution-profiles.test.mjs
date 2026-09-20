import test from 'node:test';
import assert from 'node:assert/strict';
import {profileSettings,matchingProfile,profileSummary,duplicateName,profileNameError} from '../resources/js/execution-profiles.js';
import {validCommand} from '../gateway/protocol.js';
const id='10000000-0000-4000-8000-000000000001';
const profile={id,name:'Everyday',account:{account_id:id,connection_id:id,identity_generation:1,connection_revision:2,transport:'openai_responses'},model:'model',reasoning_effort:'high',service_tier:null};
const valid=command=>validCommand({type:'command',request_id:id,request:{protocol:1,command}});
test('profile application is an independent four-setting snapshot',()=>{
 const settings=profileSettings(profile);assert.equal(Object.hasOwn(settings,'id'),false);assert.equal(Object.hasOwn(settings,'name'),false);
 const copy=structuredClone(profile);copy.name='Renamed';assert.equal(matchingProfile([copy],settings),copy);
 copy.account.identity_generation++;assert.equal(matchingProfile([copy],settings),undefined);
 settings.account.identity_generation=99;assert.equal(profile.account.identity_generation,1);
 assert.match(profileSummary(profile,[{binding:profile.account,label:'Work account',ready:true}]),/Work account/);
});
test('gateway accepts exact profile reads and revision-fenced mutations only',()=>{
 assert.equal(valid({op:'profiles',workspace:'/work'}),true);
 const command={op:'save_profile',workspace:'/work',command_id:id,expected_revision:0,profile,make_default:false};
 assert.equal(valid(command),true);
 for(const changed of [{expected_revision:-1},{make_default:'yes'},{profile:{...profile,instructions:'not allowed'}},{profile:{...profile,name:' x '}},{profile:{...profile,account:{...profile.account,connection_revision:null}}}])assert.equal(valid({...command,...changed}),false);
 for(const op of ['delete_profile','set_default_profile']){assert.equal(valid({op,workspace:'/work',command_id:id,expected_revision:1,profile_id:id}),true);assert.equal(valid({op,workspace:'/work',command_id:id,profile_id:id}),false);}
});

test('duplicate names preserve Unicode code points within the UTF-8 name limit',()=>{
 assert.equal(duplicateName('Everyday'),'Everyday copy');
 for(const name of ['a'.repeat(80),'é'.repeat(40),'🚢'.repeat(20)]){
  const copy=duplicateName(name);
  assert.ok(new TextEncoder().encode(copy).length<=80);
  assert.ok(copy.endsWith(' copy'));
  assert.equal(copy.isWellFormed(),true);
 }
});

test('profile summary explains disabled application for unavailable or stale account bindings',()=>{
 const choice={binding:profile.account,label:'Work account',ready:true};
 assert.doesNotMatch(profileSummary(profile,[choice]),/unavailable/);
 assert.match(profileSummary(profile,[{...choice,ready:false}]),/Work account · unavailable/);
 assert.match(profileSummary(profile,[]),/Provider account · unavailable/);
 assert.match(profileSummary(profile,[{...choice,binding:{...profile.account,identity_generation:2}}]),/Provider account · unavailable/);
});
test('profile names are validated against UTF-8 bytes before dispatch',()=>{
 assert.equal(profileNameError('é'.repeat(40)),null);
 assert.match(profileNameError('é'.repeat(41)),/80 UTF-8 bytes/);
 assert.equal(profileNameError('🚢'.repeat(20)),null);
 assert.match(profileNameError('🚢'.repeat(21)),/80 UTF-8 bytes/);
 assert.match(profileNameError('  '),/Enter a profile name/);
});
