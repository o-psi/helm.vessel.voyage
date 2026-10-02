// Qualification timing only; no effects, clock changes, content or process access.
export function measurementWindowAligned(startedAt,windows,{milliseconds=10000,tolerance=1000}={}){
 if(!Number.isFinite(startedAt)||!Array.isArray(windows)||!windows.length)return false;
 return windows.every(window=>Number.isFinite(window.start)&&Number.isFinite(window.end)
  &&window.end>=window.start&&Math.abs(window.start-startedAt)<tolerance
  &&Math.abs(window.end-(startedAt+milliseconds))<tolerance
  &&Math.abs(window.end-window.start-milliseconds)<tolerance);
}
export function privateBindingSnapshot(status){
 const value=status?.binding;const keys=['attachment_id','browser_id','capture_epoch','controller_epoch','document_epoch','incarnation','tab_id','viewport_epoch'];
 if(status?.mode!=='private'||!value||Object.keys(value).sort().join()!==keys.join())throw Error('valid private binding unavailable before detach');
 const uuid=v=>typeof v==='string'&&/^[a-f0-9]{8}-(?:[a-f0-9]{4}-){3}[a-f0-9]{12}$/.test(v)&&v!=='00000000-0000-0000-0000-000000000000';
 for(const key of ['attachment_id','browser_id','incarnation','tab_id'])if(!uuid(value[key]))throw Error('valid private identity unavailable');
 for(const key of ['capture_epoch','controller_epoch','document_epoch','viewport_epoch'])if(!Number.isSafeInteger(value[key])||value[key]<=0)throw Error('valid private fence unavailable');
 if(status.controller!==value.attachment_id)throw Error('initiating private owner not controller');
 return {...value};
}
export function privateReclaimQualified(prior,status,claims){
 const fresh=privateBindingSnapshot(status);
 for(const key of ['browser_id','incarnation','attachment_id'])if(fresh[key]!==prior[key])throw Error('private owner identity changed');
 if(fresh.controller_epoch<=prior.controller_epoch||fresh.capture_epoch<=prior.capture_epoch)throw Error('private control/capture fence did not advance');
 const attaches=[...claims.values()].filter(claim=>claim.action==='attach');
 if(!attaches.length||!attaches.some(claim=>claim.binding.browser_id===prior.browser_id&&claim.binding.incarnation===prior.incarnation&&claim.binding.attachment_id!==prior.attachment_id))throw Error('fresh requested Attach identity unavailable');
 return {same_private_owner:true,reattached:true,fresh_attach_request:true,fresh_control_and_capture_fences:true};
}
