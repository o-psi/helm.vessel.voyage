import test from 'node:test';
import assert from 'node:assert/strict';
import React from 'react';
import {renderToStaticMarkup} from 'react-dom/server';
import {RunStatus, workingFrame} from '../resources/react/App.tsx';
import workingStatuses from '../../helm/assets/working-statuses.json';
import {readFileSync} from 'node:fs';

function status(run: any, extra: any = {}, decisions: any[] = []) {
    return renderToStaticMarkup(React.createElement(RunStatus, {tab: {snapshot: {run, ...extra}, decisions} as any}));
}
test('conversation shows current run progress from the snapshot, including decisions', () => {
    assert.match(status({state:'accepted'}), /Starting/);
    assert.match(status({state:'running'}), /Working/);
    assert.match(status({state:'running'}, {}, [{}]), /Waiting for you/);
    assert.match(status({state:'cancel_requested'}), /Stopping/);
    assert.match(status({state:'completed', live_text:'provisional'}), /Finishing response/);
    assert.doesNotMatch(status({state:'completed'}), /Finished|Finishing/);
});
test('conversation exposes failure and unresolved cleanup without claiming completion', () => {
    const html = status({run_id:'r', state:'failed', failure_summary:'Provider unavailable'}, {
        pending_cleanup_run:'r', cleanup:{run_id:'r', phase:'pending', reason:'Cleanup uncertain', pending:['child process']},
    });
    assert.match(html,/Needs attention/);
    assert.match(html,/Provider unavailable/);
    assert.match(html,/Cleanup uncertain/);
    assert.match(html,/Waiting for: child process/);
    assert.doesNotMatch(html,/Ready to continue/);
    assert.match(status({state:'interrupted'}, {recovery_notice:'Saved conversation restored.'}), /Interrupted/);
    assert.match(status({state:'cancelled'}), /Ready to continue/);
});

test('running decoration uses the TUI word list and ten-frame spinner cadence', () => {
    assert.deepEqual(workingFrame(0), {word:workingStatuses[0], spinner:'⠋'});
    assert.deepEqual(workingFrame(9), {word:workingStatuses[0], spinner:'⠏'});
    assert.deepEqual(workingFrame(40), {word:workingStatuses[1], spinner:'⠋'});
    assert.deepEqual(workingFrame(40*workingStatuses.length), workingFrame(0));
    const html = status({state:'running'});
    assert.match(html, /working-indicator/);
    assert.match(html, /aria-label="Working"/);
    assert.match(html, /aria-hidden="true">Pondering/);
    assert.doesNotMatch(status({state:'running'}, {}, [{}]), /working-indicator/);
    assert.doesNotMatch(status({state:'running'}, {recovery_pending:true}), /working-indicator/);
    const css = readFileSync(new URL('../resources/react/style.css', import.meta.url), 'utf8');
    assert.match(css, /prefers-reduced-motion:reduce\).*?working-word/s);
});
