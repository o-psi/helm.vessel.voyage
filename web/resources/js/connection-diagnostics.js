// Deliberately fixed metadata schema: never pass payloads, tickets, URLs or error text.
const fields = new Set(['connection','generation','request_id','op','elapsed_ms','pending','code','reason','delay_ms','fresh_age_ms','refreshing','stale','busy','connected','journal_pending','unknown','error']);
export function connectionDiagnostic(event, metadata = {}) {
    const safe = {at:new Date().toISOString(),event};
    for (const [key,value] of Object.entries(metadata)) if (fields.has(key) && ['string','number','boolean'].includes(typeof value)) safe[key] = value;
    console.debug('[Helm connection]',safe);
}
