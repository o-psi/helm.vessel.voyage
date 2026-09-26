import test from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtempSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {readFileSync} from 'node:fs';
import {JSDOM} from 'jsdom';
test('Flux login rendering uses framework typography, provider button, and only one script bundle',(t)=>{
 const compiled=mkdtempSync(join(tmpdir(),'helm-flux-views-'));
 t.after(()=>rmSync(compiled,{recursive:true,force:true}));
 const code=`require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); config(['services.google.client_id'=>'fixture','services.google.client_secret'=>'fixture','services.x.client_id'=>'','services.github.client_id'=>'']); echo view('console.login')->render();`;
 const r=spawnSync('php',['-r',code],{cwd:new URL('..',import.meta.url),encoding:'utf8',env:{...process.env,VIEW_COMPILED_PATH:compiled}});assert.equal(r.status,0,r.stderr);
 const d=new JSDOM(r.stdout).window.document;
 assert.ok(d.querySelector('[data-flux-heading]'));const a=d.querySelector('a[data-flux-button]');assert.ok(a);assert.match(a.href,/\/auth\/google$/);assert.ok(a.querySelector('img[alt=""]'));assert.match(a.textContent,/Continue with Google/);
 assert.ok(d.body.classList.contains('dark:bg-zinc-800'));
 assert.equal([...d.scripts].filter(s=>/flux(?:\.min)?\.js/.test(s.src)).length,1);
});
test('shared CSS uses Flux theme instead of a parallel widget stylesheet',()=>{
 const css=readFileSync(new URL('../resources/css/app.css',import.meta.url),'utf8');assert.match(css,/livewire\/flux\/dist\/flux.css/);assert.match(css,/@custom-variant dark/);assert.doesNotMatch(css,/(?:^|\n)(?:button|input|textarea|:root)\s*\{/);
 const js=readFileSync(new URL('../resources/js/console.js',import.meta.url),'utf8');assert.doesNotMatch(js,/element\('(button|input)'/);assert.match(js,/fluxTemplate\('flux-action'/);
});
test('Setup profile, management, account and model choices render as searchable Flux controls',t=>{
 const compiled=mkdtempSync(join(tmpdir(),'helm-flux-setup-'));
 t.after(()=>rmSync(compiled,{recursive:true,force:true}));
 const code=`require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); echo view('livewire.console',['vessels'=>collect(),'tenantId'=>'flux-test'])->render();`;
 const r=spawnSync('php',['-r',code],{cwd:new URL('..',import.meta.url),encoding:'utf8',env:{...process.env,VIEW_COMPILED_PATH:compiled}});
 assert.equal(r.status,0,r.stderr);
 const d=new JSDOM(r.stdout).window.document;
 for(const id of ['edit-profile','edit-manage-profile','edit-account','edit-model']){
  const select=d.getElementById(id);
  assert.equal(select?.tagName,'UI-SELECT',`${id} is rendered by Flux`);
  assert.ok(select.hasAttribute('data-flux-select'));
  assert.equal(select.querySelector('button[data-flux-select-button]')?.type,'button',`${id} cannot submit the composer`);
  assert.ok(select.querySelector('input[placeholder]'),`${id} includes Flux search`);
  assert.ok(select.querySelector('ui-options'),`${id} has a Flux option container`);
 }
 assert.equal(d.getElementById('flux-search-option')?.content.firstElementChild.tagName,'UI-OPTION');
 for(const id of ['setup-profile-list','setup-manage-profile-list','setup-picker-list']) assert.equal(d.getElementById(id),null,`${id} custom choices are removed`);
 const signIn=d.getElementById('enrollment-link');
 assert.equal(signIn?.tagName,'A');
 assert.ok(signIn.hasAttribute('data-flux-button'),'Setup sign-in link uses Flux button styling and behavior');
});
