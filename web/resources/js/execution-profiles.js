export const sameAccount = (a, b) => Boolean(a && b && ['account_id','connection_id','identity_generation','connection_revision','transport'].every(key => a[key] === b[key]));
export function profileSettings(profile) {
    return {account:structuredClone(profile.account),model:profile.model,reasoning_effort:profile.reasoning_effort || null,service_tier:profile.service_tier || null};
}
export function profileSummary(profile, accounts = []) {
    if (!profile) return 'Choose a profile';
    const account = accounts.find(item => sameAccount(item.binding, profile.account));
    const label = account?.ready ? account.label : `${account?.label || 'Provider account'} · unavailable (edit this profile or choose another)`;
    return `${profile.model} · ${profile.reasoning_effort || 'Default thinking'} · ${profile.service_tier || 'Default tier'} · ${label}`;
}
export function matchingProfile(profiles, settings) {
    return profiles.find(profile => sameAccount(profile.account,settings?.account) && ['model','reasoning_effort','service_tier'].every(key => (profile[key] || null) === (settings?.[key] || null)));
}

export function duplicateName(name) {
    const suffix = ' copy', encoder = new TextEncoder();
    let prefix = '';
    for (const character of name) {
        if (encoder.encode(prefix + character + suffix).length > 80) break;
        prefix += character;
    }
    return prefix.trimEnd() + suffix;
}

export function profileNameError(name) {
    if (!name.trim()) return 'Enter a profile name.';
    if (new TextEncoder().encode(name.trim()).length > 80) return 'Profile names must be 80 UTF-8 bytes or fewer. Shorten the name before saving.';
    return null;
}
