//! Root constructs exact facts; paired owners cannot select a UID or login namespace.
use super::{database,service::Supervisor};
use anyhow::{Result,ensure,Context};
use serde::{Deserialize,Serialize};
use std::{path::{Path,PathBuf},process::Stdio,time::Duration,num::NonZeroU64};
use sha2::{Digest,Sha256};
use uuid::Uuid;
use voyage_protocol::{execution_identity::*,execution_review_control::*,identity_helper::*,process::*};
use voyage_storage::protected_linux::RootDirectory;

const PROVISION:&str="administrator-execution.json";
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Provision {
    schema:u32,
    pub(super) identity:ConfiguredExecutionIdentity,
    pub(super) config_path:PathBuf,
    pub(super) data_directory:PathBuf,
    pub(super) config_directory:PathBuf,
    pub(super) workspace_roots:Vec<PathBuf>,
}
fn kernel_authority()->Result<Vec<String>> {
    use std::io::Read;
    let mut bytes=Vec::new();std::fs::File::open("/proc/self/status")?.take(16385).read_to_end(&mut bytes)?;
    ensure!(bytes.len()<=16384,"host authority observation exceeds bounds");
    let text=std::str::from_utf8(&bytes)?;
    let names=["CapEff:","CapPrm:","CapInh:","CapBnd:","NoNewPrivs:"];
    let values=text.lines().filter(|line|names.iter().any(|name|line.starts_with(name))).map(str::to_owned).collect::<Vec<_>>();
    ensure!(values.len()==names.len(),"host capability observation unavailable");Ok(values)
}
fn hash(domain:&[u8],bytes:&[u8])->String{let mut hash=Sha256::new();hash.update(domain);hash.update(bytes);format!("{:x}",hash.finalize())}
pub(super) fn provision(root:&Path)->Result<Provision>{
    ensure!(unsafe{libc::getuid()}==0&&unsafe{libc::geteuid()}==0,"privileged execution supervisor required");
    let record:Provision=serde_json::from_slice(&RootDirectory::open(root)?.read(PROVISION.as_ref(),16384)?)?;
    ensure!(record.schema==1&&record.identity.enabled&&record.identity.authority==AuthorityClass::Administrator&&record.identity.uid==0&&record.workspace_roots.len()<=32&&!record.workspace_roots.is_empty(),"administrator execution was not explicitly provisioned");
    super::launch::validate_identity(&record.identity)?;
    ensure!(record.data_directory.starts_with(root.join("administrator-data"))&&record.config_directory.starts_with(root.join("administrator-config")),"administrator account/configuration must use separately provisioned control-root namespaces");
    RootDirectory::open(&record.data_directory)?.child("helm".as_ref())?.child("accounts".as_ref())?;
    RootDirectory::open(&record.config_directory)?;
    let parent=record.config_path.parent().context("administrator launch parent unavailable")?;
    let name=record.config_path.file_name().context("administrator launch name unavailable")?;
    RootDirectory::open(parent)?.read(name,65536)?;
    Ok(record)
}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct NamespacePin {schema:u32,session:Uuid,command:Uuid,identity:IdentityRef,provision_digest:String}
pub(super) fn pin_namespace(root:&Path,session:Uuid,command:Uuid,record:&Provision)->Result<()> {
    let pin=NamespacePin{schema:1,session,command,identity:record.identity.identity.clone(),provision_digest:hash(b"voyage/administrator-provision/v1\0",&serde_json::to_vec(record)?)};
    let bytes=serde_json::to_vec(&pin)?;let name=format!("{session}-{command}.json");
    let directory=RootDirectory::open(root)?.create_child("administrator-launches".as_ref())?;
    match directory.read(name.as_ref(),4096){Ok(previous)=>ensure!(previous==bytes,"administrator namespace pin changed"),Err(error) if error.downcast_ref::<std::io::Error>().is_some_and(|error|error.kind()==std::io::ErrorKind::NotFound)=>directory.publish_new(name.as_ref(),&bytes,4096)?,Err(error)=>return Err(error)};
    Ok(())
}
/// Same immutable namespace pin for the reviewed independent guardian lifetime.
pub(super) fn runtime_namespace(root:&Path,registration:&ProcessRegistration,identity:&ConfiguredExecutionIdentity)->Result<Option<(PathBuf,PathBuf)>>{
    if identity.authority!=AuthorityClass::Administrator{return Ok(None);}
    let record=provision(root)?;
    let name=format!("{}-{}.json",registration.session_id,registration.command_id);
    let pin:NamespacePin=serde_json::from_slice(&RootDirectory::open(root)?.child("administrator-launches".as_ref())?.read(name.as_ref(),4096)?)?;
    ensure!(pin.schema==1&&pin.session==registration.session_id&&pin.command==registration.command_id&&pin.identity==identity.identity&&record.identity==*identity&&pin.provision_digest==hash(b"voyage/administrator-provision/v1\0",&serde_json::to_vec(&record)?),"administrator context changed after review");
    Ok(Some((record.data_directory,record.config_directory)))
}
pub(super) async fn facts(binary:&Path,record:&Provision,workspace:&Path)->Result<IdentityConfigFacts>{
    super::launch::protected_binary(binary)?;
    let mut command=tokio::process::Command::new(binary);
    command.arg("identity-helper").current_dir(&record.identity.home).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    super::launch::configure_identity(command.as_std_mut(),&record.identity)?;
    command.env("XDG_DATA_HOME",&record.data_directory).env("XDG_CONFIG_HOME",&record.config_directory);
    let mut child=command.spawn()?;
    let result:std::result::Result<anyhow::Result<IdentityConfigFacts>,tokio::time::error::Elapsed>=tokio::time::timeout(Duration::from_secs(8),async{
        let request=IdentityHelperRequest{schema:IDENTITY_HELPER_SCHEMA,workspace:workspace.to_owned(),operation:IdentityHelperOperation::ReviewConfig{config_path:record.config_path.clone()}};
        write_frame(&mut child.stdin.take().context("helper pipe unavailable")?,&request).await?;
        let reply:IdentityHelperResponse=read_frame(&mut child.stdout.take().context("helper pipe unavailable")?).await?;
        ensure!(child.wait().await?.success(),"administrator preflight unavailable");
        let IdentityHelperResponse::Facts{facts}=reply else{anyhow::bail!("administrator account/configuration preflight unavailable")};
        ensure!(facts.uid==record.identity.uid&&facts.gid==record.identity.gid&&{let mut groups=record.identity.supplementary_groups.clone();groups.sort_unstable();groups.dedup();facts.supplementary_groups==groups},"administrator identity changed during preflight");
        Ok(facts)
    }).await;
    if let Ok(Ok(facts))=result{return Ok(facts);}
    let _=child.kill().await;anyhow::bail!("administrator preflight unavailable")
}
pub(super) fn observation(root:&Path,session:Uuid,incarnation:Uuid)->Result<ObservedExecution>{
    let record=RootDirectory::open(root)?.child("guardians".as_ref())?.child(session.to_string().as_ref())?.child(incarnation.to_string().as_ref())?;
    let observed:ObservedExecution=serde_json::from_slice(&record.read("observed.json".as_ref(),8192)?)?;
    ensure!(observed.incarnation==incarnation,"observed execution incarnation mismatch");Ok(observed)
}
impl Supervisor {
    pub(super) async fn execution_facts(&self,grant:&ConnectionGrant,review_id:Uuid,command_id:Uuid,session:Uuid,incarnation:Uuid,workspace:&Path,identity:&IdentityRef)->Result<(Provision,ReviewFacts,String)>{
        super::access::store::current_connection(&self.directory,grant)?;
        ensure!(grant.full_access&&workspace.is_absolute()&&std::fs::canonicalize(workspace)?==workspace,"execution review requires exact owner workspace");
        let record=provision(&self.directory)?;
        ensure!(record.identity.identity==*identity&&record.workspace_roots.iter().any(|root|std::fs::canonicalize(root).is_ok_and(|root|workspace.starts_with(root))),"administrator identity/workspace unavailable");
        database::store_identity(&self.directory,&record.identity).await?;
        let authority=database::execution_reviews::authority(&self.directory,grant).await?;
        let preflight=facts(&self.binary,&record,workspace).await?;
        let release=super::launch::protected_binary(&self.binary)?;
        let supervisor=super::launch::protected_binary(&self.binary.with_file_name("vessel"))?;
        let host=hash(b"voyage/administrator-host/v1\0",&serde_json::to_vec(&(&record.identity,&record.data_directory,&record.config_directory,&preflight.account_root_digest,std::fs::read_link("/proc/self/ns/user")?.as_os_str().as_encoded_bytes(),kernel_authority()?))?);
        let policy=hash(b"voyage/administrator-policy/v1\0",&serde_json::to_vec(&(&preflight.policy_digest,&preflight.config_digest,&supervisor,&record.workspace_roots))?);
        let current=ReviewFacts{vessel_id:grant.vessel_id,session_id:session,run_id:review_id,incarnation,requester_id:grant.principal_id,connection_id:grant.grant_id,connection_revision:NonZeroU64::new(grant.revision).context("connection revision unavailable")?,administrative_owner_id:grant.principal_id,authority_revision:NonZeroU64::new(authority).context("owner authority unavailable")?,expected_session_revision:0,change:ExecutionChange::Start,previous_incarnation:None,identity:identity.clone(),account_context:record.identity.account_context.clone(),account:preflight.account,account_capability_revision:preflight.capability_revision,workspace:workspace.to_owned(),host_identity_digest:host,policy_digest:policy,release_digest:release,pending_work_digest:hash(b"voyage/administrator-fresh-work/v1\0",&serde_json::to_vec(&(command_id,session,incarnation))?)};
        ensure!(provision(&self.directory)?.identity==record.identity,"administrator provision changed");
        super::access::store::current_connection(&self.directory,grant)?;
        Ok((record,current,preflight.config_digest))
    }
    pub(super) async fn execution_operation(&self,grant:&ConnectionGrant,operation:ExecutionOperation)->Result<serde_json::Value>{
        match operation {
            ExecutionOperation::Inventory=>{
                let eligible=database::execution_reviews::authority(&self.directory,grant).await.is_ok();
                let ordinary=super::default_execution::protected_default(&self.directory)?;
                let ordinary_summary=IdentitySummary{identity:ordinary.identity,label:ordinary.label,authority:ordinary.authority,account_context:ordinary.account_context,available:ordinary.enabled,unavailable_reason:None};
                let capability=match provision(&self.directory){Ok(record) if eligible=>{let available=facts(&self.binary,&record,&record.workspace_roots[0]).await.is_ok();ExecutionCapability::Available{schema:EXECUTION_SCHEMA,installation_scope:InstallationScope::System,identities:vec![ordinary_summary.clone(),IdentitySummary{identity:record.identity.identity,label:record.identity.label,authority:record.identity.authority,account_context:record.identity.account_context,available,unavailable_reason:(!available).then_some(ExecutionFailure::AccountUnavailable)}],can_review_administrator:available,can_transition:false}},_=>ExecutionCapability::Available{schema:EXECUTION_SCHEMA,installation_scope:InstallationScope::System,identities:vec![ordinary_summary],can_review_administrator:false,can_transition:false}};
                Ok(serde_json::to_value(capability)?)
            }
            ExecutionOperation::PrepareTransition{review_id,command_id,session_id,source_incarnation,identity,stop_source}=>self.prepare_execution_transition(grant,review_id,command_id,session_id,source_incarnation,identity,stop_source).await,
            ExecutionOperation::Prepare{review_id,command_id,session_id,workspace,identity}=>{
                ensure!(!review_id.is_nil()&&!command_id.is_nil()&&!session_id.is_nil(),"nil execution review identity");
                if let Ok(saved)=database::execution_reviews::resolve(&self.directory,grant,review_id).await {
                    ensure!(saved.review.command_id==command_id&&saved.review.facts.session_id==session_id&&saved.review.facts.workspace==workspace&&saved.review.facts.identity==identity,"execution preparation receipt conflict");
                    return Ok(serde_json::to_value(saved)?);
                }
                let lock=self.bound_creation_lock(session_id).await?;let _guard=lock.lock().await;
                ensure!(self.registration(session_id).await.is_err(),"existing Voyage requires reviewed identity transition");
                let (_,current,_)=self.execution_facts(grant,review_id,command_id,session_id,Uuid::new_v4(),&workspace,&identity).await?;
                let now=u64::try_from(chrono::Utc::now().timestamp_millis())?;
                let mut review=ExecutionReview{schema:EXECUTION_SCHEMA,review_id,command_id,created_at_ms:now,expires_at_ms:now+MAX_REVIEW_LIFETIME_MS,facts:current.clone(),digest:String::new()};review.digest=review.calculated_digest().map_err(|_|anyhow::anyhow!("execution review unavailable"))?;
                Ok(serde_json::to_value(database::execution_reviews::prepare(&self.directory,grant,&review,&current).await?)?)
            }
            ExecutionOperation::Approve{approval}=>{
                let saved=database::execution_reviews::resolve(&self.directory,grant,approval.review_id).await?;
                ensure!(saved.review.command_id==approval.command_id&&saved.review.digest==approval.digest,"execution approval identity conflict");
                if saved.receipt.outcome!=ExecutionOutcome::AwaitingApproval{return Ok(serde_json::to_value(saved)?);}
                if saved.review.facts.change==ExecutionChange::Transition{return self.approve_execution_transition(grant,approval,saved).await;}
                let original=&saved.review.facts;
                let lock=self.bound_creation_lock(original.session_id).await?;let _guard=lock.lock().await;
                ensure!(self.registration(original.session_id).await.is_err(),"Voyage was admitted after this review");
                let (record,current,config_digest)=self.execution_facts(grant,saved.review.review_id,saved.review.command_id,original.session_id,original.incarnation,&original.workspace,&original.identity).await?;
                let approved=database::execution_reviews::approve(&self.directory,grant,&approval,&current).await?;
                if !database::execution_reviews::mark_launching(&self.directory,grant,&approval).await?{return Ok(serde_json::to_value(approved)?);}
                pin_namespace(&self.directory,original.session_id,saved.review.command_id,&record).map_err(|error|error.context(super::routing::OutcomeUnknown))?;
                super::identity_start::pin_launch(&self.directory,saved.review.command_id,original.session_id,&record.config_path,&config_digest).map_err(|error|error.context(super::routing::OutcomeUnknown))?;
                let binding=ExecutionBinding{session_id:original.session_id,incarnation:original.incarnation,identity:original.identity.clone(),account_context:original.account_context.clone(),peer_uids:ProcessPeerUids{supervisor:0,runtime:record.identity.uid},administrator_grant_id:approved.administrator_grant_id,host_identity_digest:original.host_identity_digest.clone(),policy_digest:original.policy_digest.clone()};
                let command=VesselCommand::Execution{operation:ExecutionOperation::Approve{approval:approval.clone()}};
                let launched=self.start_bound_request_locked(saved.review.command_id,original.session_id,original.workspace.clone(),record.config_path,binding,command).await;
                let outcome=match launched {Ok(value) if value["state"]=="live"=>match observation(&self.directory,original.session_id,original.incarnation){Ok(observed) if observed.identity==original.identity&&observed.release_digest==original.release_digest=>ExecutionOutcome::Ready{observed},_=>ExecutionOutcome::Unconfirmed{cleanup_obligations:vec![original.session_id]}},_=>ExecutionOutcome::Unconfirmed{cleanup_obligations:vec![original.session_id]}};
                Ok(serde_json::to_value(database::execution_reviews::finish_launch(&self.directory,grant,approval.review_id,outcome).await?)?)
            }
            ExecutionOperation::Review{review_id}=>match database::execution_reviews::resolve(&self.directory,grant,review_id).await{Ok(saved)=>Ok(serde_json::to_value(saved)?),Err(_)=>self.observe_execution_transition(grant,review_id).await},
            ExecutionOperation::Control{control}=>Ok(serde_json::to_value(database::execution_reviews::control(&self.directory,grant,&control).await?)?),
            ExecutionOperation::Status{session_id}=>{
                let registration=self.registration(session_id).await?;
                self.execution_connection(grant,&registration,true).await?;
                self.connection_session(grant,session_id,&registration.workspace)?;
                let binding=database::execution_binding(&self.directory,session_id).await?.context("execution identity not observed on this peer")?;
                let identity=database::configured_identity(&self.directory,&binding.identity).await?;
                let administrator_authorized=match binding.administrator_grant_id{Some(id)=>database::administrator_grant(&self.directory,id).await.is_ok_and(|grant|grant.revoked_at_ms.is_none()),None=>false};
                let status=ExecutionStatus{session_id,incarnation:registration.incarnation,identity:IdentitySummary{identity:identity.identity,label:identity.label,authority:identity.authority,account_context:identity.account_context,available:identity.enabled,unavailable_reason:None},observed:observation(&self.directory,session_id,registration.incarnation).ok(),process_state:super::bound_lifecycle::inspect(&self.directory,&registration).await.state,administrator_authorized,cleanup_observed:super::guardian::cleanup_observed(&self.directory,session_id,registration.incarnation).unwrap_or(false)};
                Ok(serde_json::to_value(status)?)
            }
        }
    }
}

pub(super) async fn verify_running(root:&Path,registration:&ProcessRegistration,identity:&ConfiguredExecutionIdentity)->Result<()> {
    if identity.authority!=AuthorityClass::Administrator{return Ok(());}
    let binding=database::execution_binding(root,registration.session_id).await?.context("administrator binding unavailable")?;
    let facts=database::execution_reviews::grant_facts(root,binding.administrator_grant_id.context("administrator grant unavailable")?).await?;
    ensure!(facts.incarnation==registration.incarnation&&facts.identity==identity.identity&&facts.workspace==registration.workspace,"administrator launch review changed");
    let record=provision(root)?;runtime_namespace(root,registration,identity)?;
    let registry=voyage_runtime::accounts::Registry::new(record.data_directory.join("helm").join("accounts"));
    ensure!(registry.validate_binding(&facts.account)?.capability_revision==facts.account_capability_revision,"administrator account capability changed");
    let parent=record.config_path.parent().context("configuration parent unavailable")?;
    let bytes=RootDirectory::open(parent)?.read(record.config_path.file_name().context("configuration unavailable")?,65536)?;
    ensure!(super::identity_start::launch_digest(root,registration)?.as_deref()==Some(hash(b"voyage/identity-launch-config/v1\0",&bytes).as_str()),"administrator configuration changed after review");
    let account_root=record.data_directory.join("helm").join("accounts").canonicalize()?;
    let namespace_digest=hash(b"voyage/identity-account-namespace/v1\0",account_root.as_os_str().as_encoded_bytes());
    let host=hash(b"voyage/administrator-host/v1\0",&serde_json::to_vec(&(identity,&record.data_directory,&record.config_directory,&namespace_digest,std::fs::read_link("/proc/self/ns/user")?.as_os_str().as_encoded_bytes(),kernel_authority()?))?);
    ensure!(host==facts.host_identity_digest&&super::launch::protected_binary(registration.executable.as_deref().context("execution binary unavailable")?)?==facts.release_digest,"administrator host or release changed");
    Ok(())
}
