import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const css=readFileSync(new URL('../resources/css/console.css',import.meta.url),'utf8');
test('voyage cards retain Flux buttons with space, multiline titles and distinct selected/focus states',()=>{
 assert.match(css,/#helm-client #voyages\s*\{[^}]*gap: 0\.625rem/);
 assert.match(css,/button\[data-flux-sidebar-item\]\s*\{[^}]*min-height: 4rem/);
 assert.match(css,/padding: 0\.75rem !important/);
 assert.match(css,/-webkit-line-clamp: 2/);
 assert.match(css,/\[data-current\]\s*\{[^}]*border-color: var\(--color-emerald-500\)/);
 assert.match(css,/:focus-visible\s*\{[^}]*outline: 2px/);
 assert.match(css,/\.dark #helm-client #voyages button/);
 assert.doesNotMatch(css,/#voyages\s*\{[^}]*overflow:\s*hidden/);
});
