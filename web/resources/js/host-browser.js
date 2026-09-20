import {mountBrowserViewer} from '../../../helm/browser-view/viewer.mjs';
import {request, voyageResult} from './vessel-client.js';

// This adapter deliberately bypasses IntentJournal: even human navigation and
// SDP must never enter localStorage, diagnostics, or the conversation stream.
export function mountHostBrowser(root, {client, sessionId, incarnation, context, onClose}) {
    const transport = async operation => {
        const response = await client.exchange(request('host_browser', {session_id:sessionId, incarnation, operation}));
        return voyageResult(response, sessionId, incarnation).result;
    };
    return mountBrowserViewer(root, {transport, context, onClose});
}
