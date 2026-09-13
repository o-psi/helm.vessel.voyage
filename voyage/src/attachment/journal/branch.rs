//! Immutable owner-created branch snapshots are imported only by a destination runtime.
use super::*;
use voyage_protocol::process::RuntimeInitialization;
impl Journal {
    pub(crate) fn import_process_branch(
        &mut self,
        initialization: &RuntimeInitialization,
        workspace: &Path,
    ) -> Result<()> {
        let RuntimeInitialization::Branch {
            source_directory,
            source_session_id,
            source_command_id,
            branch_id,
        } = initialization
        else {
            anyhow::bail!("not branch initialization")
        };
        let provenance = serde_json::to_string(initialization)?;
        let imported: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_branch_import')",
            [],
            |r| r.get(0),
        )?;
        if imported {
            let prior: Option<String> = self
                .connection
                .query_row(
                    "SELECT provenance FROM process_branch_import WHERE session_id=?1",
                    [branch_id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(prior) = prior {
                ensure!(prior == provenance, "branch import provenance conflict");
                self.load_session(*branch_id)?;
                return Ok(());
            }
        }
        let source = Journal::open(source_directory.join("journal"))?;
        let (actual_source,actual_branch,payload,digest,configuration):(String,String,String,String,Option<String>)=source.connection.query_row("SELECT source_session_id,branch_id,snapshot,digest,configuration FROM process_branches WHERE command_id=?1",[source_command_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        ensure!(
            actual_source == source_session_id.to_string()
                && actual_branch == branch_id.to_string(),
            "branch provenance mismatch"
        );
        let mut hash = Sha256::new();
        hash.update(payload.as_bytes());
        if let Some(configuration) = &configuration {
            ensure!(
                configuration.len() <= 1024 * 1024,
                "branch configuration exceeds limit"
            );
            hash.update(configuration.as_bytes());
        }
        ensure!(
            payload.len() <= MAX_SNAPSHOT && hex::encode(hash.finalize()) == digest,
            "branch snapshot integrity mismatch"
        );
        let branch: Session = serde_json::from_str(&payload)?;
        ensure!(
            branch.id == *branch_id
                && branch.parent_id == Some(*source_session_id)
                && branch.workspace.canonicalize()? == workspace,
            "branch destination mismatch"
        );
        ensure!(
            branch.messages.iter().all(|m| m.provider_state.is_none())
                && branch.terminals.is_empty()
                && branch.completion_runs.is_empty()
                && branch.workflow_runs.is_empty(),
            "branch retains live runtime state"
        );
        // Persist destination-owned blobs before admitting the branch snapshot.
        // A crash leaves only bounded unreferenced blobs; the idempotent import
        // verifies/copies them again without rewriting canonical image identities.
        if branch.messages.iter().any(|m| !m.parts.is_empty()) {
            let images = crate::images::Store::open(&source.directory, *source_session_id)?;
            let mut destination = crate::images::Store::open(&self.directory, *branch_id)?;
            for part in branch.messages.iter().flat_map(|m| &m.parts) {
                if let voyage_protocol::content::ContentPart::Image { attachment } = part {
                    ensure!(
                        images.copy_to(&mut destination, attachment)? == *attachment,
                        "branch image identity changed"
                    );
                }
            }
        }
        if branch.messages.iter().any(|m| {
            m.tool_output
                .as_ref()
                .is_some_and(|o| o.artifacts().next().is_some())
        }) {
            let artifacts = crate::artifacts::Store::open(&source.directory, *source_session_id)?;
            let mut destination = crate::artifacts::Store::open(&self.directory, *branch_id)?;
            for reference in branch
                .messages
                .iter()
                .filter_map(|m| m.tool_output.as_ref())
                .flat_map(|o| o.artifacts())
            {
                ensure!(
                    artifacts.copy_to(&mut destination, reference)? == *reference,
                    "branch artifact identity changed"
                );
            }
        }
        let provenance = serde_json::to_string(initialization)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS process_branch_import(session_id TEXT PRIMARY KEY,provenance TEXT NOT NULL,digest TEXT NOT NULL)")?;
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT provenance,digest FROM process_branch_import WHERE session_id=?1",
                [branch_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((saved, hash)) = prior {
            ensure!(
                saved == provenance && hash == digest,
                "branch import provenance conflict"
            );
            return Ok(());
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
            [branch_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(!exists, "branch destination identity already exists");
        steering::validate_snapshot_ids(&tx, &branch)?;
        tx.execute(
            "INSERT INTO sessions(id,revision,state) VALUES(?1,0,?2)",
            params![branch_id.to_string(), payload],
        )?;
        if let Some(configuration) = configuration {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS process_configuration(session_id TEXT PRIMARY KEY,settings TEXT NOT NULL)")?;
            tx.execute(
                "INSERT INTO process_configuration VALUES(?1,?2)",
                params![branch_id.to_string(), configuration],
            )?;
        }
        tx.execute(
            "INSERT INTO process_branch_import VALUES(?1,?2,?3)",
            params![branch_id.to_string(), provenance, digest],
        )?;
        commit(tx, &self.commit_fence)
    }
    pub(crate) fn frozen_branch_configuration(
        initialization: Option<&RuntimeInitialization>,
    ) -> Result<Option<String>> {
        let Some(RuntimeInitialization::Branch {
            source_directory,
            source_session_id,
            source_command_id,
            branch_id,
        }) = initialization
        else {
            return Ok(None);
        };
        let source = Journal::open(source_directory.join("journal"))?;
        let (session,branch,payload,digest,configuration):(String,String,String,String,Option<String>)=source.connection.query_row("SELECT source_session_id,branch_id,snapshot,digest,configuration FROM process_branches WHERE command_id=?1",[source_command_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        ensure!(
            session == source_session_id.to_string() && branch == branch_id.to_string(),
            "frozen branch configuration identity mismatch"
        );
        let mut hash = Sha256::new();
        hash.update(payload.as_bytes());
        if let Some(settings) = &configuration {
            ensure!(
                settings.len() <= 1024 * 1024,
                "frozen branch configuration exceeds limit"
            );
            hash.update(settings.as_bytes());
        }
        ensure!(
            hex::encode(hash.finalize()) == digest,
            "frozen branch configuration integrity mismatch"
        );
        Ok(configuration)
    }
}

/// Select canonical context, never a display projection. A historical boundary
/// includes the selected saved user message, but no later assistant/tool output.
/// Validate every retained group, not merely the message adjacent to the cutoff.
pub(super) fn branch_messages(messages: &[Message], through: Option<u64>) -> Result<Vec<Message>> {
    let Some(index) = through else {
        return Ok(messages.to_vec());
    };
    let index = usize::try_from(index).context("branch message index exceeds platform range")?;
    ensure!(
        messages.get(index).is_some_and(|m| m.role == Role::User),
        "branch point must be a saved user message"
    );
    let prefix = &messages[..=index];
    let mut pending = std::collections::BTreeSet::new();
    let mut seen = std::collections::BTreeSet::new();
    for message in prefix {
        if message.role == Role::Tool {
            ensure!(
                message.tool_calls.is_empty(),
                "tool result cannot declare tool calls"
            );
            let id = message
                .tool_call_id
                .as_deref()
                .context("tool result has no call ID")?;
            ensure!(
                pending.remove(id),
                "orphan, duplicate or mismatched tool result"
            );
        } else {
            ensure!(
                pending.is_empty(),
                "branch history contains an incomplete tool group"
            );
            ensure!(
                message.tool_call_id.is_none(),
                "non-tool message has a result ID"
            );
            ensure!(
                message.tool_calls.is_empty() || message.role == Role::Assistant,
                "only assistant messages may declare tool calls"
            );
            for call in &message.tool_calls {
                ensure!(
                    !call.id.trim().is_empty() && seen.insert(call.id.as_str()),
                    "empty or duplicate tool call ID"
                );
                pending.insert(call.id.as_str());
            }
        }
    }
    ensure!(pending.is_empty(), "branch would split a tool group");
    let mut context = prefix.to_vec();
    for message in &mut context {
        message.provider_state = None;
    }
    Ok(context)
}

#[cfg(test)]
mod historical_tests {
    use super::*;
    use crate::model::ToolCall;

    fn user(text: &str) -> Message {
        Message::new(Role::User, text)
    }
    fn calls(ids: &[&str]) -> Message {
        let mut message = Message::new(Role::Assistant, "tools");
        message.tool_calls = ids
            .iter()
            .map(|id| ToolCall {
                id: (*id).into(),
                name: "read_file".into(),
                arguments: serde_json::json!({}),
            })
            .collect();
        message
    }
    #[test]
    fn exact_inclusive_canonical_prefix_and_source_unchanged() {
        let mut messages = vec![
            user("first"),
            calls(&["a", "b"]),
            Message::tool("b", "B"),
            Message::tool("a", "A"),
            user("selected"),
            Message::new(Role::Assistant, "excluded"),
            user("later"),
        ];
        messages[1].provider_state = Some(serde_json::json!({"continuation":"old"}));
        let before = serde_json::to_value(&messages).unwrap();
        let branch = branch_messages(&messages, Some(4)).unwrap();
        assert_eq!(branch.len(), 5);
        assert_eq!(branch[4].content, "selected");
        assert!(branch.iter().all(|m| m.provider_state.is_none()));
        assert_eq!(serde_json::to_value(&messages).unwrap(), before);
        assert_eq!(branch_messages(&messages, None).unwrap().len(), 7);
    }
    #[test]
    fn rejects_non_user_out_of_range_and_every_malformed_group() {
        for messages in [
            vec![calls(&["a"]), user("split")],
            vec![
                calls(&["a", "a"]),
                Message::tool("a", "x"),
                user("duplicate"),
            ],
            vec![calls(&[""]), Message::tool("", "x"), user("empty")],
            vec![calls(&["a"]), Message::tool("b", "x"), user("mismatch")],
            vec![Message::tool("a", "x"), user("orphan")],
            vec![
                calls(&["a"]),
                Message::tool("a", "x"),
                Message::tool("a", "x"),
                user("duplicate result"),
            ],
            vec![
                calls(&["a", "b"]),
                Message::tool("a", "x"),
                user("incomplete"),
                user("later"),
            ],
            vec![
                calls(&["a"]),
                Message::tool("a", "x"),
                calls(&["a"]),
                Message::tool("a", "x"),
                user("reused"),
            ],
        ] {
            assert!(branch_messages(&messages, Some((messages.len() - 1) as u64)).is_err());
        }
        assert!(branch_messages(&[user("ok")], Some(u64::MAX)).is_err());
        assert!(branch_messages(&[calls(&[])], Some(0)).is_err());
        assert!(branch_messages(&[], Some(0)).is_err());
    }
    #[test]
    fn validates_role_metadata_and_ignores_excluded_suffix() {
        let mut malformed = user("wrong");
        malformed.tool_call_id = Some("a".into());
        assert!(branch_messages(&[malformed], Some(0)).is_err());
        let mut malformed = calls(&["a"]);
        malformed.role = Role::User;
        assert!(branch_messages(&[malformed], Some(0)).is_err());
        let mut result = Message::tool("a", "x");
        result.tool_calls = calls(&["b"]).tool_calls;
        assert!(branch_messages(&[calls(&["a"]), result, user("end")], Some(2)).is_err());
        assert_eq!(
            branch_messages(&[user("selected"), calls(&["unfinished"])], Some(0))
                .unwrap()
                .len(),
            1
        );
    }
}

#[cfg(test)]
mod journal_cutoff_tests {
    use super::*;
    use voyage_protocol::process::RuntimeCommand;
    #[test]
    fn snapshot_import_cutoff_revision_deduplication_and_identity() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        let workspace = root.path().canonicalize().unwrap();
        let mut journal = Journal::open(source.join("journal")).unwrap();
        let mut session = Session::new(workspace.clone(), "fixture".into());
        session.messages = vec![
            Message::new(Role::User, "first"),
            Message::new(Role::Assistant, "answer"),
            Message::new(Role::User, "selected"),
            Message::new(Role::Assistant, "excluded"),
        ];
        journal.create_session(&session).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        journal.initialize_lifecycle(&guard).unwrap();
        journal.initialize_process_commands(&guard).unwrap();
        let before =
            serde_json::to_value(&journal.load_session(session.id).unwrap().session).unwrap();
        let command_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let now = chrono::Utc::now().timestamp_millis();
        let command = RuntimeCommand::Branch {
            command_id,
            expected_revision: 0,
            expires_at_ms: (now + 60000) as u64,
            branch_id,
            name: None,
            through_message: Some(2),
        };
        let mut stale = command.clone();
        if let RuntimeCommand::Branch {
            expected_revision, ..
        } = &mut stale
        {
            *expected_revision = 1;
        }
        assert!(
            journal
                .apply_lifecycle(&guard, &stale, now, Some("{}"))
                .is_err()
        );
        let receipt = journal
            .apply_lifecycle(&guard, &command, now, Some("{}"))
            .unwrap();
        assert_eq!(receipt["through_message"], 2);
        assert_eq!(receipt["message_count"], 3);
        assert_eq!(
            journal
                .apply_lifecycle(&guard, &command, now, Some("{}"))
                .unwrap(),
            receipt
        );
        let mut conflict = command.clone();
        if let RuntimeCommand::Branch {
            through_message, ..
        } = &mut conflict
        {
            *through_message = Some(0);
        }
        assert!(
            journal
                .apply_lifecycle(&guard, &conflict, now, Some("{}"))
                .is_err()
        );
        let mut after =
            serde_json::to_value(&journal.load_session(session.id).unwrap().session).unwrap();
        // Lifecycle receipt advances the revision; canonical history is unchanged.
        after["revision"] = before["revision"].clone();
        assert_eq!(after, before);
        let destination = root.path().join("destination");
        std::fs::create_dir(&destination).unwrap();
        let mut dest = Journal::open(destination.join("journal")).unwrap();
        let init = RuntimeInitialization::Branch {
            source_directory: source,
            source_session_id: session.id,
            source_command_id: command_id,
            branch_id,
        };
        dest.import_process_branch(&init, &workspace).unwrap();
        let branch = dest.load_session(branch_id).unwrap().session;
        assert_ne!(branch.id, session.id);
        assert_eq!(branch.parent_id, Some(session.id));
        assert_eq!(branch.workspace, workspace);
        assert_eq!(branch.messages.len(), 3);
        assert_eq!(branch.messages[2].content, "selected");
        assert_eq!(branch.working_context.generation, 0);
        dest.import_process_branch(&init, &workspace).unwrap();
        assert_eq!(
            dest.load_session(branch_id).unwrap().session.messages.len(),
            3
        );
    }
}
