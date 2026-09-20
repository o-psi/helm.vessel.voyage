import {request,uuid} from '../js/vessel-client.js';
export async function vesselRead(connection:any,op:string,fields:Record<string,unknown>={}) {
    const client=connection?.client;if(!client)throw new Error('Vessel is offline.');
    const response=await client.exchange(request(op,fields));
    if(connection.client!==client||response?.protocol!==1||response.outcome_unknown!==false||response.error!=null)throw new Error('Vessel could not confirm this request. Reload choices.');
    return response.result;
}
export function accountChoices(catalogue:any) {
    return (catalogue.accounts||[]).flatMap((account:any)=>{
        const provider=catalogue.connections?.find((connection:any)=>connection.id===account.connection_id);
        return (provider?.transports||[]).map((transport:string)=>({label:`${account.label} · ${provider.label} · ${transport.replaceAll('_',' ')}`,ready:account.state==='ready'&&account.availability==='available',binding:{account_id:account.id,connection_id:provider.id,identity_generation:account.identity_generation,connection_revision:provider.revision,transport}}));
    });
}
// Creation intent is separate from message submission and is never automatically replayed.
export class Creation {
    constructor(private storage:Storage,private tenant:string) {}
    prefix(){return `helm-web:start:${this.tenant}:`;}
    pending(){const entries=[];for(let i=0;i<this.storage.length;i++){const key=this.storage.key(i);if(key?.startsWith(this.prefix()))entries.push({key,...JSON.parse(this.storage.getItem(key)!)});}return entries;}
    async start(connection:any,workspace:string,settings:any) {
        const locks = globalThis.navigator?.locks;
        if (locks) return locks.request(`${this.prefix()}${connection.id}:${connection.vessel_id}`, {ifAvailable:true}, lock => { if(!lock)throw new Error('Another tab is creating a voyage. Check creation before trying again.'); return this.startLocked(connection,workspace,settings); });
        return this.startLocked(connection,workspace,settings);
    }
    private async startLocked(connection:any,workspace:string,settings:any) {
        if(this.pending().some(record=>record.vessel===connection.id))throw new Error('Creation unconfirmed. Check creation before trying again.');
        const command={op:'start_account',command_id:uuid(),session_id:uuid(),workspace:workspace==='/'?workspace:workspace.replace(/\/+$/,''),...settings};
        const key=`${this.prefix()}${connection.id}:${connection.vessel_id}:${command.command_id}`;
        this.storage.setItem(key,JSON.stringify({vessel:connection.id,vessel_id:connection.vessel_id,command}));
        const response=await connection.client.exchange({protocol:1,command});
        if(response?.protocol!==1||response.outcome_unknown!==false)throw new Error('Creation uncertain. Check creation; do not resend.');
        if(response.error!=null){this.storage.removeItem(key);throw new Error('Creation refused. Review settings.');}
        return this.accept(connection,key,command,response.result);
    }
    // The matching start reply/resolve receipt proves the request identity. Vessel
    // canonicalizes paths (including symlinks); lexical equality is not authority.
    accept(connection:any,key:string,command:any,process:any){
        if(process?.session_id!==command.session_id||(typeof process.workspace!=='string'||!process.workspace.startsWith('/'))||typeof process.incarnation!=='string')throw new Error('Creation identity unconfirmed. Check creation.');
        this.storage.removeItem(key);connection.lastCatalogue=0;
        if(!connection.voyages.some((voyage:any)=>voyage.session_id===process.session_id))connection.voyages.unshift(process);
        return process;
    }
    async reconcile(connection:any,record:any){
        if(connection.vessel_id!==record.vessel_id)throw new Error('Vessel identity changed.');
        const result=await vesselRead(connection,'resolve_start_account',{...record.command,op:'resolve_start_account'});
        if(result.command_id!==record.command.command_id||result.session_id!==record.command.session_id)throw new Error('Creation receipt identity changed.');
        if(result.status==='created')return this.accept(connection,record.key,record.command,result.process);
        if(result.status==='not_admitted'){this.storage.removeItem(record.key);return null;}
        throw new Error('Creation remains uncertain. Nothing replayed.');
    }
}
