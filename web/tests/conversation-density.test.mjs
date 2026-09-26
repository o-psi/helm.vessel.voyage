import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const css=readFileSync(new URL('../resources/css/console.css',import.meta.url),'utf8');
test('wide desktop conversation and composer share a viewport-responsive width',()=>{
 assert.match(css,/@media \(min-width: 1024px\)/);
 assert.match(css,/max-width: 80rem/);
 assert.match(css,/@media \(min-width: 1920px\)/);
 assert.match(css,/max-width: 96rem/);
 for(const id of ['messages','live-previews','live-output','conversation-empty','composer'])assert.ok(css.includes(`#${id}`));
 const rules=css.replace(/\/\*[\s\S]*?\*\//g,'');
 const widths=[...rules.matchAll(/#helm-client :is\(#messages, #live-previews, #live-output, #conversation-empty, #composer\)\s*\{([^}]*)\}/g)];
 assert.equal(widths.length,2);
 for(const [,declarations] of widths)assert.doesNotMatch(declarations,/\b(?:zoom|transform)\s*:/);
 assert.doesNotMatch(rules,/@media[^{]*\b(?:device-pixel-ratio|resolution)\b/);
});
test('compact prose is scoped to the log and live output, not controls or global Flux text',()=>{
 assert.match(css,/#helm-client #messages :is\(p, li, blockquote, td, th, \[data-code\], \[data-quote\]\)/);
 assert.match(css,/#helm-client #output-text\s*\{\s*font-size: 0\.875rem;\s*line-height: 1\.6;/);
 assert.doesNotMatch(css,/(?:^|\n)(?:body|html|p|button|textarea|\[data-flux-text\])\s*\{/);
});
