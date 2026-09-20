import {mountBrowserViewer} from '../../../helm/browser-view/viewer.mjs';
import {request, voyageResult} from './vessel-client.js';

const prepared = value => value?.status === 'prepared' && value?.not_dispatched === true;

// Ephemeral only: no IntentJournal, localStorage, diagnostics or conversation data.
export function hostBrowserAdapter({client, sessionId, incarnation, context}) {
    let current = null;
    const readContext = () => current ?? {...context(), incarnation};
    const exchange = async (operation, owner) => voyageResult(await client.exchange(
        request('host_browser', {session_id:sessionId, incarnation:owner, operation})), sessionId);
    const transport = async operation => {
        const owner = readContext().incarnation;
        const mayPrepare = operation.action === 'status' || operation.action === 'start';
        if (operation.binding && operation.binding.incarnation !== owner ||
            operation.action === 'start' && operation.incarnation !== owner) throw Error('Browser owner mismatch');
        const reply = await exchange(operation, owner);
        if (!prepared(reply.result)) {
            if (reply.incarnation !== owner) throw Error('Browser owner changed; not replayed');
            return reply.result;
        }
        if (!mayPrepare || operation.binding) throw Error('Unexpected browser preparation');
        const snapshot = voyageResult(await client.exchange(request('snapshot', {
            session_id:sessionId, incarnation:reply.incarnation,
        })), sessionId, reply.incarnation).result;
        if (!Number.isSafeInteger(snapshot?.revision) || snapshot.revision < 0) throw Error('Browser snapshot revision unavailable');
        current = {incarnation:reply.incarnation, revision:snapshot.revision};
        // Verified non-admission: this exact intent never reached the owner. Keep
        // its command ID, changing only the newly observed owner/revision fences.
        const retry = operation.action === 'start'
            ? {...operation, incarnation:current.incarnation, expected_revision:current.revision}
            : operation;
        const result = await exchange(retry, current.incarnation);
        if (result.incarnation !== current.incarnation || prepared(result.result)) throw Error('Repeated preparation or owner changed; not replayed');
        return result.result;
    };
    return {transport, context:readContext};
}

export function mountHostBrowser(root, options) {
    return mountBrowserViewer(root, {...hostBrowserAdapter(options), onClose:options.onClose});
}
