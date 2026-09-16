//! Vessel-owned unsent content. Execution envelopes remain private Helm recovery state.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use voyage_protocol::{content::ContentPart, vessel::VesselCommand};

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Link {
    pub id: Option<Uuid>,
    pub revision: u64,
    pub base: Option<serde_json::Value>,
    pub target: Option<serde_json::Value>,
    pub conflict: bool,
    #[serde(default)]
    pub fork: bool,
    #[serde(default)]
    pub send_revision: Option<u64>,
    pub admitted_revision: Option<u64>,
}
#[derive(Default)]
pub(super) struct State {
    busy: bool,
    next: Option<std::time::Instant>,
    pub notice: String,
    job: Option<tokio::task::JoinHandle<Vec<Result<Batch>>>>,
}
#[derive(Clone)]
pub(super) struct Entry {
    pub(super) local: Destination,
    pub(super) link: Link,
    pub(super) document: serde_json::Value,
    pub(super) composer: composer::Composer,
    pub(super) images: Vec<attachments::Image>,
    pub(super) frozen: bool,
    pub(super) authored: serde_json::Value,
    pub(super) clearing: bool,
}
#[derive(Clone, Copy)]
pub(super) enum Destination {
    New(Uuid),
    Live(Target),
}
struct Batch {
    route: state::Route,
    entries: Vec<Entry>,
    discovered: Vec<Entry>,
}

fn mutation_id(id: Uuid, operation: &str, value: &serde_json::Value) -> Uuid {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(serde_json::to_vec(&(id, operation, value)).expect("JSON value"));
    Uuid::from_bytes(hash[..16].try_into().expect("digest length"))
}
async fn request(client: &Client, operation: serde_json::Value) -> Result<serde_json::Value> {
    // Deserialize the public envelope: no client-specific opaque draft format crosses the wire.
    client
        .request(serde_json::from_value::<VesselCommand>(
            serde_json::json!({
                "op":"drafts", "operation":operation
            }),
        )?)
        .await
}
pub(super) fn document(
    target: serde_json::Value,
    composer: &composer::Composer,
    images: &[attachments::Image],
) -> Result<serde_json::Value> {
    Ok(serde_json::json!({"target":target,"parts":attachments::content(composer, images)?}))
}
pub(super) fn same(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    // Staged image ids differ from private upload identities; content hashes and order do not.
    fn normalized(mut v: serde_json::Value) -> serde_json::Value {
        if let Some(parts) = v["parts"].as_array_mut() {
            for part in parts {
                if part["type"] == "image" {
                    part["attachment"]["id"] = serde_json::Value::Null;
                }
            }
        }
        v
    }
    normalized(a.clone()) == normalized(b.clone())
}
async fn restore(client: &Client, record: &serde_json::Value, local: Destination) -> Result<Entry> {
    let id: Uuid = serde_json::from_value(record["draft_id"].clone())?;
    let parts: Vec<ContentPart> = serde_json::from_value(record["document"]["parts"].clone())?;
    let mut composer = composer::Composer::default();
    let mut images = Vec::new();
    for part in parts {
        match part {
            ContentPart::Text { text } => composer.insert_str(&text),
            ContentPart::Image { attachment } => {
                let mut offset = 0u64;
                let mut bytes = Vec::new();
                loop {
                    let page = request(client, serde_json::json!({"op":"read_image","draft_id":id,"attachment_id":attachment.id,"offset":offset,"limit":262144})).await?;
                    bytes.extend(
                        STANDARD.decode(
                            page["data_base64"]
                                .as_str()
                                .context("draft image bytes missing")?,
                        )?,
                    );
                    anyhow::ensure!(
                        bytes.len() <= attachments::MAX_BYTES,
                        "draft image exceeds limit"
                    );
                    match page["next_offset"].as_u64() {
                        Some(next) => {
                            anyhow::ensure!(next > offset, "invalid image continuation");
                            offset = next;
                        }
                        None => break,
                    }
                }
                let image = attachments::Image::from_bytes(attachment.name.clone(), &bytes)?;
                anyhow::ensure!(
                    image.metadata().sha256 == attachment.sha256,
                    "draft image changed during restore"
                );
                composer.insert_image(image.metadata().id);
                images.push(image);
            }
        }
    }
    Ok(Entry {
        local,
        document: record["document"].clone(),
        authored: record["document"].clone(),
        clearing: false,
        composer,
        images,
        frozen: false,
        link: Link {
            id: Some(id),
            revision: record["revision"]
                .as_u64()
                .context("draft revision missing")?,
            base: Some(record["document"].clone()),
            target: Some(record["document"]["target"].clone()),
            ..Default::default()
        },
    })
}
async fn sync(client: Client, route: state::Route, mut entries: Vec<Entry>) -> Result<Batch> {
    let listing = request(&client, serde_json::json!({"op":"list"})).await?;
    let records = listing["drafts"]
        .as_array()
        .context("draft catalogue missing")?;
    let mut used = std::collections::BTreeSet::new();
    for entry in &mut entries {
        let remote = records
            .iter()
            .find(|record| {
                entry
                    .link
                    .id
                    .is_some_and(|id| record["draft_id"] == id.to_string())
            })
            .or_else(|| {
                records.iter().find(|record| {
                    if let Some(id) = entry.link.id.filter(|_| {
                        entry.link.revision > 0 || matches!(entry.local, Destination::New(_))
                    }) {
                        record["draft_id"] == id.to_string()
                    } else {
                        !entry.link.fork
                            && !matches!(entry.local, Destination::New(_))
                            && record["document"]["target"] == entry.document["target"]
                    }
                })
            });
        if let Some(remote) = remote {
            used.insert(remote["draft_id"].as_str().unwrap_or_default().to_owned());
        }
        if let (Some(id), Some(revision)) = (entry.link.id, entry.link.admitted_revision) {
            let cleared = serde_json::json!({"target":entry.link.target,"parts":[]});
            match request(&client, serde_json::json!({"op":"put","command_id":mutation_id(id,"clear",&serde_json::json!([revision,cleared])),"draft_id":id,"expected_revision":revision,"document":cleared})).await {
                Ok(record) => {
                    entry.link.revision = record["revision"].as_u64().context("clear revision missing")?;
                    entry.link.base = Some(cleared);
                    entry.link.admitted_revision = None;
                    entry.link.send_revision = None;
                    // New chats and steering have consumed intent, not an after-run queue.
                    // Keep later authored content but create a fresh reviewed intent locally.
                    if entry.link.target.as_ref().is_some_and(|target| target["type"] != "message") {
                        entry.link = Link { fork:true, ..Default::default() };
                    }
                }
                Err(_) => { entry.link.conflict = true; }
            }
            continue;
        }
        if entry.frozen {
            continue;
        }
        if let Some(remote) = remote {
            let changed = entry
                .link
                .base
                .as_ref()
                .is_none_or(|base| !same(base, &entry.document));
            let empty = entry.composer.text.is_empty() && entry.images.is_empty();
            if entry.link.revision == 0
                && !empty
                && !same(&remote["document"], &entry.document)
                && !remote["document"]["parts"]
                    .as_array()
                    .is_some_and(|parts| parts.is_empty())
            {
                entry.link.conflict = true;
                continue;
            }
            if entry.link.revision == 0
                && !empty
                && remote["document"]["parts"]
                    .as_array()
                    .is_some_and(|parts| parts.is_empty())
            {
                entry.link.id = Some(serde_json::from_value(remote["draft_id"].clone())?);
                entry.link.revision = remote["revision"]
                    .as_u64()
                    .context("draft revision missing")?;
                entry.link.base = Some(remote["document"].clone());
            }
            if entry.link.revision == 0 || remote["revision"].as_u64() != Some(entry.link.revision)
            {
                if changed && entry.link.revision > 0 && !same(&remote["document"], &entry.document)
                {
                    entry.link.conflict = true;
                    continue;
                }
                let authored = entry.authored.clone();
                *entry = restore(&client, remote, entry.local).await?;
                entry.authored = authored;
                continue;
            }
        } else if entry.link.id.is_some() && entry.link.revision > 0 {
            // Deletion elsewhere is a conflict, never permission to resurrect or overwrite.
            entry.link.conflict = true;
            continue;
        }
        if entry.link.conflict
            || entry
                .link
                .base
                .as_ref()
                .is_some_and(|base| same(base, &entry.document))
        {
            continue;
        }
        if entry.composer.text.is_empty() && entry.images.is_empty() && entry.link.revision == 0 {
            continue;
        }
        let id = entry.link.id.unwrap_or_else(Uuid::new_v4);
        let mut revision = entry.link.revision;
        if entry.link.revision == 0 {
            let initial = serde_json::json!({"target":entry.document["target"],"parts":[]});
            let record = request(&client, serde_json::json!({"op":"put","command_id":mutation_id(id, "create", &initial),"draft_id":id,"expected_revision":0,"document":initial})).await?;
            revision = record["revision"]
                .as_u64()
                .context("draft create revision missing")?;
            entry.link.id = Some(id);
        }
        let mut parts = attachments::content(&entry.composer, &entry.images)?;
        for part in &mut parts {
            if let ContentPart::Image { attachment } = part {
                let image = entry
                    .images
                    .iter()
                    .find(|image| image.metadata().id == attachment.id)
                    .context("image missing")?;
                *attachment = serde_json::from_value(request(&client, serde_json::json!({"op":"upload_image","command_id":mutation_id(id, "image", &serde_json::json!(attachment)),"draft_id":id,"name":attachment.name,"data_base64":STANDARD.encode(image.preview_bytes()?)})).await?)?;
            }
        }
        let doc = serde_json::json!({"target":entry.document["target"],"parts":parts});
        match request(&client, serde_json::json!({"op":"put","command_id":mutation_id(id, &revision.to_string(), &doc),"draft_id":id,"expected_revision":revision,"document":doc})).await {
            Ok(record) => {
                entry.link.revision = record["revision"].as_u64().context("draft save revision missing")?;
                entry.link.base = Some(doc);
                entry.link.target = Some(entry.document["target"].clone());
            }
            Err(_) => { entry.link.conflict = true; }
        }
    }
    let mut discovered = Vec::new();
    for record in records {
        if record["document"]["target"]["type"] == "new_chat"
            && record["document"]["parts"]
                .as_array()
                .is_some_and(|parts| !parts.is_empty())
            && !used.contains(record["draft_id"].as_str().unwrap_or_default())
        {
            discovered.push(restore(&client, record, Destination::New(Uuid::new_v4())).await?);
        }
    }
    Ok(Batch {
        route,
        entries,
        discovered,
    })
}

impl App {
    pub(super) fn poll_shared_drafts(&mut self) {
        // Capture intention as soon as authored content exists, not at delayed network save.
        for (target, view) in &mut self.views {
            if view.shared.target.is_none()
                && (!view.draft.text.is_empty() || !view.images.is_empty())
            {
                view.shared.target = Some(
                    if let Some(run) = view
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.run.as_ref())
                        .filter(|r| r.active())
                    {
                        serde_json::json!({"type":"steer","session_id":target.session,"run_id":run.run_id,"incarnation":view.process.incarnation})
                    } else {
                        serde_json::json!({"type":"message","session_id":target.session})
                    },
                );
                let _ = drafts::save(&self.clients[target.route], view);
            }
        }

        if self
            .shared_drafts
            .job
            .as_ref()
            .is_some_and(|job| job.is_finished())
        {
            let job = self.shared_drafts.job.take().unwrap();
            match futures_util::FutureExt::now_or_never(job) {
                Some(Ok(batches)) => {
                    self.shared_drafts.notice = "Vessel drafts checked · autosave active".into();
                    for result in batches {
                        let batch = match result {
                            Ok(batch) => batch,
                            Err(_) => {
                                self.shared_drafts.notice = "Drafts offline / unavailable · private recovery retained; retrying".into();
                                continue;
                            }
                        };
                        if !self.clients.current(batch.route) {
                            continue;
                        }
                        for mut entry in batch.entries.into_iter().chain(batch.discovered) {
                            if entry.link.conflict {
                                self.shared_drafts.notice = "Draft conflict · edits retained · Ctrl+Alt+L keeps local copy; Ctrl+Alt+R restores Vessel".into();
                            }
                            match entry.local {
                                Destination::Live(target) => {
                                    if let Some(view) = self.views.get_mut(&target) {
                                        if view.pending.is_some()
                                            || (!entry.clearing
                                                && view.shared.id != entry.link.id
                                                && view.shared.fork)
                                        {
                                            continue;
                                        }
                                        if view.shared.admitted_revision.is_some()
                                            && !entry.clearing
                                        {
                                            continue;
                                        }
                                        let current = document(
                                            entry.authored["target"].clone(),
                                            &view.draft,
                                            &view.images,
                                        )
                                        .ok();
                                        let clean = current
                                            .as_ref()
                                            .is_some_and(|doc| same(doc, &entry.authored));
                                        if !clean && !same(&entry.authored, &entry.document) {
                                            entry.link.conflict = true;
                                        }
                                        if clean && !entry.link.conflict {
                                            view.draft = entry.composer;
                                            view.images = entry.images;
                                        }
                                        if !clean {
                                            self.shared_drafts.notice =
                                                "Draft has newer local edits · saving again".into();
                                        }
                                        view.shared = entry.link;
                                        let _ = drafts::save(&self.clients[target.route], view);
                                    }
                                }
                                Destination::New(id) => {
                                    if self.apply_shared_new(id, batch.route, entry).is_err() {
                                        self.shared_drafts.notice = "Shared draft restore could not be persisted · local recovery retained".into();
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {
                    self.shared_drafts.notice =
                        "Drafts offline / unavailable · private local recovery retained; retrying"
                            .into()
                }
            }
            self.shared_drafts.busy = false;
            self.shared_drafts.next = Some(std::time::Instant::now() + Duration::from_secs(3));
        }
        if self.shared_drafts.busy
            || self
                .shared_drafts
                .next
                .is_some_and(|next| next > std::time::Instant::now())
        {
            return;
        }
        let mut work = Vec::new();
        for (target, view) in &mut self.views {
            if view.shared.id.is_none() && (!view.draft.text.is_empty() || !view.images.is_empty())
            {
                view.shared.id = Some(Uuid::new_v4());
                if drafts::save(&self.clients[target.route], view).is_err() {
                    self.shared_drafts.notice =
                        "Private recovery could not be saved; synchronization paused".into();
                    return;
                }
            }
        }
        self.prepare_shared_new();
        for client in self
            .clients
            .iter()
            .filter(|client| self.clients.available(state::Route::of(client)))
        {
            let route = state::Route::of(client);
            let mut entries = self.shared_new_entries(route);
            for (target, view) in &self.views {
                if target.route != route {
                    continue;
                }
                let intent = view.shared.target.clone().unwrap_or_else(|| {
                    if let Some(run) = view.snapshot.as_ref().and_then(|s| s.run.as_ref()).filter(|r| r.active()) {
                        serde_json::json!({"type":"steer","session_id":target.session,"run_id":run.run_id,"incarnation":view.process.incarnation})
                    } else { serde_json::json!({"type":"message","session_id":target.session}) }
                });
                if let Ok(doc) = document(intent, &view.draft, &view.images) {
                    entries.push(Entry {
                        local: Destination::Live(*target),
                        link: view.shared.clone(),
                        clearing: view.shared.admitted_revision.is_some(),
                        authored: doc.clone(),
                        document: doc,
                        composer: view.draft.clone(),
                        images: view.images.clone(),
                        frozen: view.pending.is_some(),
                    });
                }
            }
            work.push((client.clone(), route, entries));
        }
        self.shared_drafts.busy = true;
        self.shared_drafts.job = Some(tokio::spawn(async move {
            let mut results = Vec::new();
            for (client, route, entries) in work {
                results.push(sync(client, route, entries).await);
            }
            results
        }));
    }
    pub(super) fn shared_send_guard(&self, target: Target) -> Result<()> {
        let view = &self.views[&target];
        anyhow::ensure!(
            !view.shared.conflict,
            "Resolve the draft conflict before sending (Ctrl+Alt+L keeps local; Ctrl+Alt+R restores Vessel)"
        );
        if view.shared.id.is_some() {
            let doc = document(
                view.shared.target.clone().context("Draft intent missing")?,
                &view.draft,
                &view.images,
            )?;
            anyhow::ensure!(
                view.shared
                    .base
                    .as_ref()
                    .is_some_and(|base| same(base, &doc)),
                "Draft autosave pending; wait for Vessel synchronization before sending"
            );
        }
        if let Some(intent) = &view.shared.target {
            if intent["type"] == "steer" {
                let run = view
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.run.as_ref())
                    .filter(|r| r.active());
                anyhow::ensure!(
                    run.is_some_and(|run| intent["run_id"] == run.run_id.to_string())
                        && intent["incarnation"] == view.process.incarnation.to_string(),
                    "Steering draft targets an earlier run; retained without sending. Ctrl+Alt+L explicitly copies it for the current target"
                );
            } else {
                anyhow::ensure!(
                    !view
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.run.as_ref())
                        .is_some_and(|r| r.active()),
                    "Message draft is not steering. Ctrl+Alt+L explicitly copies it for the current run"
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutation_identity_is_exact_and_stable() {
        let id = Uuid::new_v4();
        let value = serde_json::json!({"parts":[{"type":"text","text":"draft"}]});
        assert_eq!(mutation_id(id, "2", &value), mutation_id(id, "2", &value));
        assert_ne!(mutation_id(id, "2", &value), mutation_id(id, "3", &value));
        assert_ne!(
            mutation_id(id, "2", &value),
            mutation_id(id, "2", &serde_json::json!({"parts":[]}))
        );
    }
    #[test]
    fn normalized_content_preserves_order_and_target_not_local_image_identity() {
        let a = serde_json::json!({"target":{"type":"steer","run_id":"one"},"parts":[{"type":"image","attachment":{"id":"local","sha256":"hash"}},{"type":"text","text":"later"}]});
        let mut b = a.clone();
        b["parts"][0]["attachment"]["id"] = serde_json::json!("vessel");
        assert!(same(&a, &b));
        b["target"]["run_id"] = serde_json::json!("two");
        assert!(!same(&a, &b));
        b = a.clone();
        b["parts"].as_array_mut().unwrap().reverse();
        assert!(!same(&a, &b));
    }
    #[test]
    fn private_recovery_is_not_a_shared_document() {
        let mut composer = composer::Composer::default();
        composer.insert_str("review me");
        let doc = document(
            serde_json::json!({"type":"message","session_id":Uuid::new_v4()}),
            &composer,
            &[],
        )
        .unwrap();
        assert_eq!(doc.as_object().unwrap().len(), 2);
        assert!(doc.get("pending").is_none());
        assert!(doc.get("command_id").is_none());
        assert!(doc.get("start").is_none());
    }
}

impl State {
    pub(super) async fn finish(&mut self) {
        if let Some(job) = self.job.take() {
            job.abort();
            let _ = job.await;
        }
    }
}

#[cfg(test)]
mod race_tests {
    use super::*;
    fn text(value: &str) -> serde_json::Value {
        serde_json::json!({"target":{"type":"message","session_id":Uuid::nil()},"parts":[{"type":"text","text":value}]})
    }
    #[test]
    fn later_local_edits_are_not_the_generation_captured_by_save_or_clear() {
        let captured = text("sent");
        let later = text("later");
        assert!(!same(&captured, &later));
        let clear = serde_json::json!({"target":captured["target"],"parts":[]});
        assert!(!same(&clear, &later));
        assert_eq!(clear["target"], captured["target"]);
    }
    #[test]
    fn steering_target_cannot_normalize_to_another_run_or_to_message() {
        let mut a = text("steer");
        a["target"] = serde_json::json!({"type":"steer","session_id":Uuid::nil(),"run_id":Uuid::new_v4(),"incarnation":Uuid::new_v4()});
        let mut b = a.clone();
        b["target"]["run_id"] = serde_json::json!(Uuid::new_v4());
        assert!(!same(&a, &b));
        assert!(!same(&a, &text("steer")));
    }
}

#[cfg(all(test, unix))]
#[path = "shared_drafts_tests.rs"]
mod socket_tests;
