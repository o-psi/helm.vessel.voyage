//! Minimal private dispatch/cleanup evidence, without action arguments, browser text or pixels.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize,Serialize};
use std::path::{Path,PathBuf};
use uuid::Uuid;
use voyage_protocol::{browser::*,vessel::VoyageCommand};
use crate::{attachment::local_actor::storage::Directory,process_client::transport::Client};

#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Resource { version:u32, connection_id:Uuid, binding:BrowserBinding }
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Dispatch {
    pub binding:BrowserBinding,
    pub request_id:Uuid,
    pub action_sha256:String,
    pub local_dispatch_possible:bool,
}
pub(super) fn offer(root:&Path,client:&Client,binding:&BrowserBinding)->Result<()> {
    Directory::open_existing(root)?.publish("binding.json",&serde_json::to_vec(&Resource {version:1,connection_id:client.id(),binding:binding.clone()})?)?;
    Ok(())
}
pub(super) fn record(root:&Path,record:&Dispatch)->Result<()> {
    Directory::open_existing(root)?.publish(&format!("action-{}.json",record.request_id),&serde_json::to_vec(record)?)?;
    Ok(())
}
pub(super) fn cleanup(root:&Path)->Result<()> {
    Directory::open_existing(root)?.publish("cleanup.json",br#"{"observed":true}"#)?;
    Ok(())
}
/// Explicit reconciliation only. Sends cleanup proof; never repeats an action or invents success.
pub(crate) async fn reconcile(client:Client,session:Uuid,incarnation:Uuid)->Result<String> {
    let mut count=0usize;
    for root in resources()? {
        let directory=Directory::open_existing(&root)?;
        let Some(bytes)=directory.read_bounded("binding.json",65_536)? else {continue;};
        let resource:Resource=serde_json::from_slice(&bytes)?;
        if resource.version!=1 || resource.connection_id!=client.id() || resource.binding.session_id!=session {continue;}
        let closed=directory.read_bounded("cleanup.json",1024)?.is_some_and(|b|b==br#"{"observed":true}"#);
        for entry in std::fs::read_dir(&root)? {
            let name=entry?.file_name();let Some(name)=name.to_str() else {continue;};
            if !name.starts_with("action-")||!name.ends_with(".json") {continue;}
            let bytes=directory.read_bounded(name,65_536)?.context("Local browser dispatch record disappeared")?;
            let record:Dispatch=serde_json::from_slice(&bytes)?;
            if record.local_dispatch_possible && !closed {continue;}
            ensure!(record.binding.session_id==session && record.binding.executor_id==resource.binding.executor_id,"Local browser evidence identity mismatch");
            let operation=BrowserOperation::Cleanup {command_id:Uuid::new_v4(),binding:record.binding,request_id:record.request_id,observed:true};
            client.voyage(session,incarnation,VoyageCommand::Browser {operation}).await?;
            count+=1;
            ensure!(count<=1024,"Browser reconciliation limit reached; inspect retained resources before another batch");
        }
    }
    Ok(format!("Reconciled {count} browser cleanup records from positive local evidence. No browser effects were replayed or reported successful. Unobserved resources remain fenced."))
}
fn resources()->Result<Vec<PathBuf>> {
    let mut roots=Vec::new();
    for entry in std::fs::read_dir(super::assets::root()?)? {
        let entry=entry?;
        if entry.file_name().to_str().is_some_and(|n|n.starts_with("session-")) {
            ensure!(roots.len()<1024,"Local browser resource retention limit reached; review retained profiles");
            roots.push(entry.path());
        }
    }
    Ok(roots)
}
