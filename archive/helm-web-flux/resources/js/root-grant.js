// Filesystem consent is distinct from generic tool approval. Authority remains in Voyage.
export function renderRootGrant(request, {heading, content, actions, button, respond, host}) {
    if (request?.kind !== 'root_grant') return false;
    heading.textContent = 'Filesystem access requested';
    const grant = request.root_grant;
    if (!grant || typeof grant.path !== 'string' || !grant.path.startsWith('/') ||
        /[\u0000-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/u.test(grant.path) ||
        !['read', 'write'].includes(grant.permission) || grant.lifetime !== 'current_run' ||
        typeof grant.reason !== 'string' || !grant.reason.trim()) {
        content.textContent = 'Unsupported filesystem request. No access granted. Refresh or use an updated native Helm client.';
        return true;
    }
    content.textContent = `Executing Vessel: ${host}\nDirectory: ${grant.path}\nPermission: ${grant.permission === 'write' ? 'Read and write' : 'Read only'}\nLifetime: Current run only\nNot inherited by children or already-running processes.\nReason: ${grant.reason}`;
    actions.append(
        button('Grant access for this run', () => respond({root_grant: 'approved'})),
        button('Deny access', () => respond({root_grant: 'denied'})),
    );
    return true;
}
