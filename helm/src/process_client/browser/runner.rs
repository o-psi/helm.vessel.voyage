//! A browser lease is pinned to one socket lifetime. Reconnect never restores sharing.
use super::{Control, Status, assets, helper::Helper, journal};
use crate::process_client::transport::Client;
use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use voyage_protocol::{
    browser::*,
    vessel::{VesselCommand, VoyageCommand},
};

fn now() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}
fn control(value: &Value) -> BrowserControl {
    match value["mode"].as_str() {
        Some("agent") if value["shared"] == true => BrowserControl::Shared,
        Some("human") => BrowserControl::Human,
        _ => BrowserControl::Private,
    }
}
async fn exchange(
    client: &Client,
    binding: &BrowserBinding,
    operation: BrowserOperation,
) -> Result<BrowserReply> {
    serde_json::from_value(
        client
            .voyage(
                binding.session_id,
                binding.incarnation,
                VoyageCommand::Browser { operation },
            )
            .await?,
    )
    .context("Invalid browser broker reply")
}
async fn set_control(
    client: &Client,
    binding: &BrowserBinding,
    value: BrowserControl,
) -> Result<()> {
    exchange(
        client,
        binding,
        BrowserOperation::Control {
            command_id: Uuid::new_v4(),
            binding: binding.clone(),
            control: value,
        },
    )
    .await?;
    Ok(())
}

pub(super) async fn run(
    client: Client,
    session: Uuid,
    incarnation: Uuid,
    label: String,
    mut commands: mpsc::Receiver<Control>,
    stop: CancellationToken,
    status: &watch::Sender<Status>,
) -> Result<()> {
    let assets = assets::distribution()?;
    if !assets::installed(&assets) {
        status.send_modify(|s| {
            s.summary =
                "Browser adapter not installed. Run `helm browser setup`, then reopen Browser"
                    .into()
        });
        anyhow::bail!("Browser adapter not installed");
    }
    let executable = assets::executable()?;
    // Establish the actual existing duplex route before any local resource admission.
    client.request(VesselCommand::Capabilities).await?;
    let mut connection = client.connection_state();
    let initial_socket = *connection.borrow();
    ensure!(
        initial_socket.socket_id.is_some(),
        "Browser sharing requires the authenticated full-duplex Vessel connection"
    );
    let root = assets::root()?.join(format!("session-{}", Uuid::new_v4()));
    let private = crate::attachment::local_actor::storage::Directory::open(&root)?;
    let _ownership = private.lock()?;
    status.send_modify(|s|s.summary="Starting local Chromium: requires Node 24+, installed executable and a working Chromium sandbox".into());
    let helper = Helper::start(&assets.join("helper.mjs")).await?;
    let mut notices = helper.events();
    let mut binding = BrowserBinding {
        session_id: session,
        incarnation,
        run_id: None,
        browser_id: Uuid::new_v4(),
        resource_id: Uuid::new_v4(),
        executor_id: Uuid::new_v4(),
        controller_epoch: 1,
        capture_epoch: 1,
        expires_at_ms: now() + 30_000,
    };
    let result = async {
        let init = helper.call(json!({"op":"init","session_dir":root,"executable_path":executable,
            "browser_id":binding.browser_id,"resource_id":binding.resource_id,"executor_id":binding.executor_id,
            "label":label,"heartbeat_ms":5000}), Duration::from_secs(30)).await?;
        let companion = super::launcher(&root, init["companion_url"].as_str().context("Companion address unavailable")?)?;
        // Helper may mint its own live browser identity. It becomes fixed before the first offer.
        binding.browser_id = serde_json::from_value(init["browser_id"].clone())?;
        binding.controller_epoch = init["epoch"].as_u64().context("Browser controller epoch unavailable")?;
        binding.capture_epoch = init["capture_epoch"].as_u64().unwrap_or(binding.controller_epoch);
        status.send_modify(|s| { s.launcher=Some(companion); s.summary="Private local browser ready. Open companion, review origins and explicitly Share / Return to agent".into(); });
        let control_stop = stop.clone();
        let control_helper = helper.clone();
        let controller = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _=control_stop.cancelled()=> {
                        let _=control_helper.call(json!({"op":"control","mode":"private"}),Duration::from_secs(2)).await;
                        break;
                    }
                    command=commands.recv()=> {
                        let Some(command)=command else { break; };
                        if control_helper.call(json!({"op":"control","mode":match command {Control::Human=>"human",Control::Private=>"private"}}),Duration::from_secs(2)).await.is_err() { break; }
                    }
                }
            }
        });
        journal::offer(&root,&client,&binding)?;
        let mut offered = false;
        let mut current_control = BrowserControl::Private;
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_heartbeat = tokio::time::Instant::now()-Duration::from_secs(3);
        let mut last_renewal = tokio::time::Instant::now();
        let mut action: Option<tokio::task::JoinHandle<Result<()>>> = None;
        let action_stop = CancellationToken::new();
        let mut pending: Option<tokio::task::JoinHandle<Result<BrowserReply>>> = None;
        let work_ready=Arc::new(tokio::sync::Notify::new());
        let mut pending_needed=true;
        let mut event_worker:Option<tokio::task::JoinHandle<Result<()>>>=None;
        let stopped = helper.stopped();
        let outcome: Result<()> = async {
            loop {
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    changed = connection.changed() => {
                        ensure!(changed.is_ok() && *connection.borrow() == initial_socket,
                            "Vessel socket changed or disconnected; local sharing requires explicit restart");
                    }
                    _ = work_ready.notified() => {pending_needed=true;},
                    _ = stopped.cancelled() => anyhow::bail!("Local browser process stopped; effects may be unknown"),
                    event = notices.recv() => {
                        match event {
                            Ok(value) if value["event"] == "control" => {
                                update_control(&client,&helper,&value,&mut binding,&mut offered,&mut current_control,status,&root,initial_socket).await?;
                                pending_needed=true;
                                last_renewal=tokio::time::Instant::now();
                            }
                            Ok(value) if value["event"] == "approval" => {
                                if value["pending"] == true { status.send_modify(|s| s.summary="Browser action awaits local confirmation in the companion".into()); }
                                else if current_control==BrowserControl::Shared {status.send_modify(|s|s.summary="Browser operation is completing; local permissions remain enforced".into());}
                            }
                            Ok(_) => (),
                            Err(_) => anyhow::bail!("Browser control observation lost; sharing fenced"),
                        }
                    }
                    _ = tick.tick() => {
                        ensure!(*connection.borrow() == initial_socket, "Vessel socket changed; sharing fenced");
                        if last_heartbeat.elapsed()>=Duration::from_secs(1) {
                            // A successful socket command proves the remote grant is still current.
                            if offered && last_renewal.elapsed()>=Duration::from_secs(2) {
                                binding.expires_at_ms=now()+10_000;
                                set_control(&client,&binding,current_control).await?;
                                last_renewal=tokio::time::Instant::now();
                            }
                            let heartbeat=if offered {json!({"op":"heartbeat","binding":binding})} else {json!({"op":"heartbeat"})};
                            let local=helper.call(heartbeat,Duration::from_secs(2)).await?;
                            update_control(&client,&helper,&local,&mut binding,&mut offered,&mut current_control,status,&root,initial_socket).await?;
                            last_heartbeat=tokio::time::Instant::now();
                        }
                        if offered && event_worker.is_none() {
                            let c=client.clone();let b=binding.clone();let notify=work_ready.clone();
                            event_worker=Some(tokio::spawn(async move {
                                let mut events=c.events(vec![voyage_protocol::vessel::VesselEventSubscription {session_id:b.session_id,incarnation:b.incarnation,after:0}]).await?;
                                // Initial recovery closes the race between Offer and subscription.
                                notify.notify_one();
                                while let Some(event)=events.next().await {
                                    let event=event?;
                                    ensure!(event.error.is_none() && event.session_id==b.session_id && event.incarnation==b.incarnation,"Browser notification owner/authority changed");
                                    if event.result["replay_gap"]==true || event.result["events"].as_array().is_some_and(|a|a.iter().any(|e|e["kind"]=="browser")) {notify.notify_one();}
                                }
                                anyhow::bail!("Browser event subscription ended; sharing fenced")
                            }));
                        }
                        if event_worker.as_ref().is_some_and(|job|job.is_finished()) {
                            event_worker.take().unwrap().await.context("Browser notification task failed")??;
                            anyhow::bail!("Browser notification worker stopped");
                        }
                        if action.as_ref().is_some_and(|job|job.is_finished()) {
                            action.take().unwrap().await.context("Browser dispatch task failed")??;
                            if current_control==BrowserControl::Shared {status.send_modify(|s|s.summary="Agent sharing enabled. Browser results remain in the Voyage conversation".into());}
                            pending_needed=true;
                        }
                        if pending.as_ref().is_some_and(|job|job.is_finished()) {
                            let reply=pending.take().unwrap().await.context("Browser pending observation task failed")??;
                            if let BrowserReply::Pending { requests }=reply
                                && let Some(request)=requests.into_iter().next().filter(|r|current_control==BrowserControl::Shared
                                    && r.binding.controller_epoch==binding.controller_epoch && r.binding.capture_epoch==binding.capture_epoch) {
                                    ensure!(action.is_none(), "Browser action arbitration conflict");
                                    let (c,h,s)=(client.clone(),helper.clone(),action_stop.clone());
                                    let expected=binding.clone();
                                    let evidence_root=root.clone();
                                    action=Some(tokio::spawn(async move { dispatch(c,h,expected,request,s,evidence_root,initial_socket).await }));
                            }
                        }
                        if offered && current_control==BrowserControl::Shared && action.is_none() && pending.is_none() && pending_needed {
                            pending_needed=false;
                            let c=client.clone();let b=binding.clone();
                            pending=Some(tokio::spawn(async move { exchange(&c,&b,BrowserOperation::Pending {binding:b.clone(),limit:1}).await }));
                        }
                    }
                }
            }
            Ok(())
        }.await;
        // Local fence first, then inform remote. The latter can fail during a partition.
        action_stop.cancel();
        controller.abort();
        let _=controller.await;
        let local_fence=helper.call(json!({"op":"control","mode":"private"}),Duration::from_secs(2)).await;
        if let Ok(value)=&local_fence {
            binding.controller_epoch=value["epoch"].as_u64().unwrap_or(binding.controller_epoch);
            binding.capture_epoch=value["capture_epoch"].as_u64().unwrap_or(binding.controller_epoch);
        }
        if let Some(job)=pending { job.abort(); let _=job.await; }
        if let Some(job)=event_worker {job.abort();let _=job.await;}
        if let Some(mut job)=action
            && tokio::time::timeout(Duration::from_secs(5), &mut job).await.is_err() {
            job.abort();let _=job.await;
            // Remote dispatched receipt remains unresolved; dropping this future is not cleanup proof.
        }
        if offered {
            let _=tokio::time::timeout(Duration::from_secs(3),set_control(&client,&binding,BrowserControl::Disconnected)).await;
        }
        outcome.and(local_fence.map(|_|()))
    }.await;
    let cleaned = helper.shutdown().await;
    private.verify()?;
    if cleaned.is_ok() {
        journal::cleanup(&root)?;
    }
    // Profile and minimal receipts are retained for explicit local recovery, never silently deleted.
    result.and(cleaned)
}

#[allow(clippy::too_many_arguments)] // Each independent authority/observation fence is explicit.
async fn update_control(
    client: &Client,
    helper: &Arc<Helper>,
    value: &Value,
    binding: &mut BrowserBinding,
    offered: &mut bool,
    current: &mut BrowserControl,
    status: &watch::Sender<Status>,
    root: &std::path::Path,
    socket: crate::process_client::duplex::ConnectionState,
) -> Result<()> {
    let epoch = value["epoch"]
        .as_u64()
        .context("Browser control epoch unavailable")?;
    let capture = value["capture_epoch"].as_u64().unwrap_or(epoch);
    ensure!(
        epoch >= binding.controller_epoch && capture >= binding.capture_epoch,
        "Browser control epoch regressed"
    );
    let next = control(value);
    let changed =
        epoch != binding.controller_epoch || capture != binding.capture_epoch || next != *current;
    binding.controller_epoch = epoch;
    binding.capture_epoch = capture;
    binding.expires_at_ms = now() + 10_000;
    if !*offered && next == BrowserControl::Shared {
        let (_, incarnation) = client
            .voyage_observed(
                binding.session_id,
                binding.incarnation,
                VoyageCommand::PrepareBrowser,
            )
            .await?;
        ensure!(
            *client.connection_state().borrow() == socket,
            "Vessel connection changed while preparing browser sharing"
        );
        binding.incarnation = incarnation;
        journal::offer(root, client, binding)?;
        let local = helper
            .call(
                json!({"op":"heartbeat","binding":binding}),
                Duration::from_secs(2),
            )
            .await?;
        ensure!(
            control(&local) == BrowserControl::Shared
                && local["epoch"].as_u64() == Some(binding.controller_epoch),
            "Local sharing changed before offer"
        );
        exchange(
            client,
            binding,
            BrowserOperation::Offer {
                command_id: Uuid::new_v4(),
                binding: binding.clone(),
            },
        )
        .await?;
        *offered = true;
    } else if *offered && changed {
        set_control(client, binding, next).await?;
    }
    *current = next;
    if changed {
        status.send_modify(|s| s.summary=match next {
        BrowserControl::Shared=>"Agent sharing enabled for this voyage. Take over or enter Private in the local companion",
        BrowserControl::Human=>"Human control. New agent actions and observations are fenced",
        _=>match value["fence_reason"].as_str() {
            Some("remote_cancellation")=>"Private: remote action cancellation fenced input and capture",
            Some("connection_liveness_expired"|"binding_expired")=>"Private: browser connection or lease liveness expired",
            Some("controller_disconnected")=>"Private: local companion controller disconnected",
            _=>"Private interaction. Agent observation and capture are suspended",
        },
    }.into());
    }
    Ok(())
}

async fn dispatch(
    client: Client,
    helper: Arc<Helper>,
    expected: BrowserBinding,
    request: BrowserRequest,
    stop: CancellationToken,
    root: std::path::PathBuf,
    socket: crate::process_client::duplex::ConnectionState,
) -> Result<()> {
    let mut connection = client.connection_state();
    ensure!(
        *connection.borrow() == socket,
        "Browser socket changed before claim"
    );
    let b = &request.binding;
    ensure!(
        b.session_id == expected.session_id
            && b.incarnation == expected.incarnation
            && b.browser_id == expected.browser_id
            && b.resource_id == expected.resource_id
            && b.executor_id == expected.executor_id
            && b.run_id.is_some()
            && b.controller_epoch == expected.controller_epoch
            && b.capture_epoch == expected.capture_epoch
            && request.expires_at_ms > now(),
        "Stale or mismatched browser request"
    );
    let mut evidence = journal::Dispatch {
        binding: b.clone(),
        request_id: request.request_id,
        action_sha256: request.action_sha256.clone(),
        local_dispatch_possible: false,
    };
    journal::record(&root, &evidence)?;
    let claim = exchange(
        &client,
        b,
        BrowserOperation::Claim {
            command_id: Uuid::new_v4(),
            binding: b.clone(),
            request_id: request.request_id,
            action_sha256: request.action_sha256.clone(),
        },
    )
    .await;
    let claimed = matches!(claim,Ok(BrowserReply::Receipt {receipt}) if receipt.state==BrowserRequestState::Dispatched);
    if !claimed || *connection.borrow() != socket || stop.is_cancelled() {
        // No local dispatch occurred. Receipt recovery, not a replacement Claim or action.
        let _ = exchange(
            &client,
            b,
            BrowserOperation::Cleanup {
                command_id: Uuid::new_v4(),
                binding: b.clone(),
                request_id: request.request_id,
                observed: true,
            },
        )
        .await;
        // Takeover/cancellation may legitimately refuse a pending claim. The socket watcher
        // separately fences connection loss; do not close a healthy human-controlled browser.
        return Ok(());
    }
    evidence.local_dispatch_possible = true;
    journal::record(&root, &evidence)?;
    let operation = json!({"op":"action","epoch":b.controller_epoch,"capture_epoch":b.capture_epoch,
        "binding":b,"expires_at_ms":request.expires_at_ms,"action_sha256":request.action_sha256,"action":request.action});
    let call = helper.call_exact(
        operation,
        request.request_id.to_string(),
        Duration::from_secs(65),
    );
    tokio::pin!(call);
    let mut receipt_tick = tokio::time::interval(Duration::from_secs(1));
    let mut cancellation_sent = false;
    let local = loop {
        tokio::select! {
            biased;
            _=stop.cancelled()=>break None,
            _=connection.changed()=>break None,
            value=&mut call=>break Some(value),
            _=receipt_tick.tick(),if !cancellation_sent=> {
                // Follow the exact in-flight receipt to propagate remote cancellation/access
                // invalidation. This read shares the same socket and never dispatches work.
                let receipt=exchange(&client,b,BrowserOperation::Receipt {binding:b.clone(),request_id:request.request_id}).await;
                let receipt_state=match &receipt {Ok(BrowserReply::Receipt{receipt})=>format!("{:?}",receipt.state),Ok(_)=>"invalid_reply".into(),Err(_)=>"unavailable".into()};
                if !matches!(receipt,Ok(BrowserReply::Receipt{receipt}) if receipt.state==BrowserRequestState::Dispatched) {
                    crate::attachment::local_actor::storage::Directory::open_existing(&root)?.publish(&format!("cancel-{}.json",request.request_id),&serde_json::to_vec(&json!({"request_id":request.request_id,"reason":receipt_state}))?)?;
                    let _=helper.call(json!({"op":"cancel","request_id":request.request_id}),Duration::from_secs(2)).await;
                    cancellation_sent=true;
                }
            }
        }
    };
    let locally_quiescent = matches!(&local,Some(Ok(value)) if value["cleanup_observed"]==true);
    let mut result = BrowserResult {
        request_id: request.request_id,
        action_sha256: request.action_sha256.clone(),
        state: BrowserRequestState::Unresolved,
        text: "Browser action outcome unknown; do not replay".into(),
        page_id: None,
        observation_id: None,
        image: None,
        file: None,
    };
    if let Some(Ok(envelope)) = local {
        if envelope["ok"] == true {
            if let Ok(value) = serde_json::from_value::<BrowserResult>(envelope["result"].clone()) {
                ensure!(
                    value.request_id == request.request_id
                        && value.action_sha256 == request.action_sha256,
                    "Browser result identity mismatch"
                );
                result = value;
            }
        } else {
            // Adapter action errors before queue admission are explicit refusals. Once admitted,
            // its drain path returns a typed result (including unresolved effects), not ok:false.
            result.state = BrowserRequestState::Refused;
            result.text = "Local browser refused the action before dispatch; review companion sharing and permissions".into();
        }
    } else {
        let _ = helper
            .call(
                json!({"op":"cancel","request_id":request.request_id}),
                Duration::from_secs(2),
            )
            .await;
    }
    if stop.is_cancelled() {
        result.image = None;
        result.file = None;
        result.page_id = None;
        result.observation_id = None;
        result.text = "Browser sharing interrupted; observations withheld".into();
    }
    let unknown_but_quiescent =
        locally_quiescent && result.state == BrowserRequestState::Unresolved;
    // Never resend uncertain results automatically. Runtime keeps the original action obligation.
    exchange(
        &client,
        b,
        BrowserOperation::Result {
            command_id: Uuid::new_v4(),
            binding: b.clone(),
            result,
        },
    )
    .await?;
    if unknown_but_quiescent {
        exchange(
            &client,
            b,
            BrowserOperation::Cleanup {
                command_id: Uuid::new_v4(),
                binding: b.clone(),
                request_id: request.request_id,
                observed: true,
            },
        )
        .await?;
    }
    Ok(())
}
