import test from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {readFileSync} from 'node:fs';
import {JSDOM} from 'jsdom';
test('Flux login rendering uses framework typography, provider button, and only one script bundle',()=>{
 const code=`require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); config(['services.google.client_id'=>'fixture','services.google.client_secret'=>'fixture','services.x.client_id'=>'','services.github.client_id'=>'']); echo view('console.login')->render();`;
 const r=spawnSync('php',['-r',code],{cwd:new URL('..',import.meta.url),encoding:'utf8'});assert.equal(r.status,0,r.stderr);
 const d=new JSDOM(r.stdout).window.document;
 assert.ok(d.querySelector('[data-flux-heading]'));const a=d.querySelector('a[data-flux-button]');assert.ok(a);assert.match(a.href,/\/auth\/google$/);assert.ok(a.querySelector('img[alt=""]'));assert.match(a.textContent,/Continue with Google/);
 assert.ok(d.body.classList.contains('dark:bg-zinc-800'));
 assert.equal([...d.scripts].filter(s=>/flux(?:\.min)?\.js/.test(s.src)).length,1);
});
test('shared CSS uses Flux theme instead of a parallel widget stylesheet',()=>{
 const css=readFileSync(new URL('../resources/css/app.css',import.meta.url),'utf8');assert.match(css,/livewire\/flux\/dist\/flux.css/);assert.match(css,/@custom-variant dark/);assert.doesNotMatch(css,/(?:^|\n)(?:button|input|textarea|:root)\s*\{/);
 const js=readFileSync(new URL('../resources/js/console.js',import.meta.url),'utf8');assert.doesNotMatch(js,/element\('(button|input)'/);assert.match(js,/fluxTemplate\('flux-action'/);
});
