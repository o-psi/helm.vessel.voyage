import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {JSDOM} from 'jsdom';
import {setComposerDisabled} from '../resources/js/composer-disabled.js';

const flux = readFileSync(new URL('../vendor/livewire/flux-pro/dist/flux.js',import.meta.url),'utf8');
const settle = () => new Promise(resolve=>setTimeout(resolve,20));
test('real Flux releases action buttons after repeated disabled renders and subsequent cycles',async()=>{
 const dom=new JSDOM('<ui-composer disabled><textarea></textarea><button id="location">Location</button><button id="account" disabled>Account</button></ui-composer>',{runScripts:'outside-only',pretendToBeVisual:true,url:'https://console.example'});
 try {
  const w=dom.window;
  w.ResizeObserver=class {observe(){} unobserve(){} disconnect(){}};
  w.matchMedia=()=>({matches:false,addEventListener(){},removeEventListener(){}});
  w.document.adoptedStyleSheets=[]; w.CSSStyleSheet.prototype.replaceSync=function(){};
  w.eval(flux);
  await settle();
  const composer=w.document.querySelector('ui-composer'),location=w.document.querySelector('#location');
  for(let cycle=0;cycle<3;cycle++) {
   setComposerDisabled(composer,true);await settle();
   setComposerDisabled(composer,true);await settle();
   assert.equal(location.disabled,true);
   setComposerDisabled(composer,false);await settle();
   location.disabled=false;await settle();
   assert.equal(location.disabled,false,'Flux must release durable disabled ownership');
   assert.equal(composer.querySelector('textarea').disabled,false);
  }
 } finally {dom.window.close();}
});
