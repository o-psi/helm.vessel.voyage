import test from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtempSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {JSDOM} from 'jsdom';

test('compact profile trigger keeps escaped identity and POST logout inside its popover', (t) => {
 const compiled=mkdtempSync(join(tmpdir(),'helm-profile-views-'));
 t.after(()=>rmSync(compiled,{recursive:true,force:true}));
 const code = `require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); auth()->setUser(new App\\Models\\User(['name'=>'Alice <script>alert(1)</script>','email'=>'alice@example.test'])); session()->put('_token','profile-test-token'); echo view('console.profile-menu')->render();`;
 const r=spawnSync('php',['-r',code],{cwd:new URL('..',import.meta.url),encoding:'utf8',env:{...process.env,VIEW_COMPILED_PATH:compiled}});
 assert.equal(r.status,0,r.stderr+r.stdout);
 const d=new JSDOM(r.stdout).window.document;
 const trigger=d.querySelector('[data-flux-profile]');
 assert.equal(trigger.getAttribute('aria-label'),'Profile menu');
 assert.equal(trigger.textContent.trim(),'A');
 assert.equal(trigger.querySelectorAll('svg, img').length,0);
 assert.match(trigger.querySelector('[data-flux-avatar]').className,/size-6/);
 const popover=d.querySelector('[popover]');
 assert.ok(popover);
 assert.match(popover.textContent,/Alice <script>alert\(1\)<\/script>/);
 assert.match(popover.textContent,/alice@example.test/);
 assert.equal(popover.querySelector('script'),null);
 const appearance=popover.querySelector('[x-model="$flux.appearance"]');
 assert.ok(appearance, 'appearance uses the shared persistent Flux preference');
 assert.deepEqual([...appearance.querySelectorAll('ui-radio')].map(r=>r.getAttribute('value')), ['light','dark','system']);
 const options=[...appearance.querySelectorAll('ui-radio')];
 assert.deepEqual(options.map(r=>r.getAttribute('aria-label')), ['Light','Dark','System']);
 for (const option of options) {
  assert.equal(option.textContent.trim(), '', 'icon-only appearance options fit the compact popover');
  assert.ok(option.querySelector('svg'));
  assert.equal(option.title, option.getAttribute('aria-label'));
 }
 assert.match(popover.textContent,/Appearance/);
 assert.match(popover.querySelector('a').href,/\/connections$/);
 const form=popover.querySelector('form');
 assert.equal(form.method,'post');
 assert.match(form.action,/\/console\/logout$/);
 assert.equal(form.querySelector('[name="_token"]').value,'profile-test-token');
 assert.equal(form.querySelector('button').type,'submit');
});
