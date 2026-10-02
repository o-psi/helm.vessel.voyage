// Qualification oracle only; no effects, credentials, content or guessed states.
import assert from 'node:assert/strict';
export function idleWindowQualified(proof,{label,milliseconds=10000}={}){
 assert.equal(proof.schema,1);assert.equal(proof.phase,'idle');assert.equal(proof.label,label);
 for(const key of ['browser_running','zero_viewers_observed','recorder_stop_ack_observed','worker_identity_unchanged','private_metadata_identity_unchanged','slot_retained','browser_root_identity_unchanged'])assert.equal(proof[key],true);
 assert.ok(Number.isFinite(proof.started_at_ms)&&Number.isFinite(proof.ended_at_ms));
 assert.ok(proof.ended_at_ms-proof.started_at_ms>=milliseconds&&proof.ended_at_ms-proof.started_at_ms<milliseconds+2000);
 assert.ok(Number.isSafeInteger(proof.samples)&&proof.samples>=30);
 return {observed:true,scope:proof.scope,label:proof.label,milliseconds:proof.ended_at_ms-proof.started_at_ms,samples:proof.samples};
}
export function idleReconnectQualified(prior,current,claims,beforeEffects,afterEffects){
 assert.ok(prior&&current?.running&&current?.binding);
 for(const key of ['browser_id','incarnation'])assert.equal(current.binding[key],prior[key]);
 const attaches=claims.filter(claim=>claim.action==='attach');
 assert.ok(attaches.some(claim=>claim.binding.browser_id===prior.browser_id&&claim.binding.incarnation===prior.incarnation&&claim.binding.attachment_id!==prior.attachment_id));
 for(const key of ['browser_starts','browser_closes']){assert.ok(Number.isSafeInteger(beforeEffects?.[key])&&beforeEffects[key]>=0);assert.equal(afterEffects?.[key],beforeEffects[key]);}
 return {same_running_browser:true,fresh_attach:true,no_start_or_close:true};
}
