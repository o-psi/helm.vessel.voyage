import test from 'node:test';
import assert from 'node:assert/strict';
import React from 'react';
import {renderToStaticMarkup} from 'react-dom/server';
import {RunStatus} from '../resources/react/App.tsx';

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
