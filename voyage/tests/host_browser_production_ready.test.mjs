import test from 'node:test';
import assert from 'node:assert/strict';
import {loaded} from './host_browser_production.mjs';

// A readiness marker expresses visible content, not a unique action target.
// Model strict locator semantics at this boundary, including duplicate headings.
function fixture(nodes){
 const calls=[];
 const locator=values=>({
  filter(options){assert.deepEqual(options,{visible:true});calls.push('visible');return locator(values.filter(value=>value.visible));},
  first(){calls.push('first');return locator(values.slice(0,1));},
  async waitFor(options){assert.deepEqual(options,{state:'visible'});assert.equal(values.length,1,'strict locator requires one element');assert.equal(values[0].visible,true);calls.push(values[0].id);}
 });
 const page={frameLocator(selector){assert.equal(selector,'.browser-next-mirror iframe');return {locator(selector){assert.equal(selector,'h1');return locator(nodes);}};}};
 return {page,calls};
}
test('public site readiness accepts duplicated visible headings',async()=>{
 const f=fixture([{id:'first',visible:true},{id:'second',visible:true}]);
 await loaded(f.page,{ready_selector:'h1'});
 assert.deepEqual(f.calls,['visible','first','first']);
});
test('public site readiness selects visible content after a hidden heading',async()=>{
 const f=fixture([{id:'hidden',visible:false},{id:'shown',visible:true}]);
 await loaded(f.page,{ready_selector:'h1'});
 assert.deepEqual(f.calls,['visible','first','shown']);
});
test('public site readiness still refuses absent or hidden markers',async()=>{
 for(const nodes of [[],[{id:'hidden',visible:false}]])await assert.rejects(loaded(fixture(nodes).page,{ready_selector:'h1'}),/strict locator requires one element/);
});
