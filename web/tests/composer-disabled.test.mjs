import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {JSDOM} from 'jsdom';
import {setComposerDisabled} from '../resources/js/composer-disabled.js';

const flux = readFileSync(new URL('../vendor/livewire/flux-pro/dist/flux.js',import.meta.url),'utf8');
const settle = () => new Promise(resolve=>setTimeout(resolve,20));
test('real Flux allows independently controlled location while message input locks and unlocks',async()=>{
 const dom=new JSDOM('<ui-composer><textarea readonly></textarea><button id="location">Location</button><button id="account" disabled>Account</button></ui-composer>',{runScripts:'outside-only',pretendToBeVisual:true,url:'https://console.example'});
 try {
  const w=dom.window;
  w.ResizeObserver=class {observe(){} unobserve(){} disconnect(){}};
  w.matchMedia=()=>({matches:false,addEventListener(){},removeEventListener(){}});
  w.document.adoptedStyleSheets=[]; w.CSSStyleSheet.prototype.replaceSync=function(){};
  w.eval(flux);
  await settle();
  const composer=w.document.querySelector('ui-composer'),location=w.document.querySelector('#location');
  for(let cycle=0;cycle<3;cycle++) {
   setComposerDisabled(composer,true);location.disabled=true;
   setComposerDisabled(composer,true);location.disabled=true;
   await settle();
   assert.equal(location.disabled,true);
   assert.equal(composer.querySelector('textarea').readOnly,true);
   setComposerDisabled(composer,false);location.disabled=false;
   await settle();
   assert.equal(location.disabled,false,'Flux must not override application button state');
   assert.equal(composer.querySelector('textarea').readOnly,false);
   // Offline drafts may lock input while still allowing a location switch.
   setComposerDisabled(composer,true);location.disabled=false;await settle();
   assert.equal(location.disabled,false);
   assert.equal(composer.hasAttribute('disabled'),false);
   assert.equal(composer.contains(location),true);

  }
 } finally {dom.window.close();}
});
