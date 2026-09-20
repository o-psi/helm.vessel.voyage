//! Shared action discovery. This catalogue never copies private input into a composer.
use super::{App, inference::Destination};
use anyhow::Result;

pub(super) const COMMANDS: &[(&str, &str, &str)] = &[
    ("help", "Show focus-specific help", ""),
    ("actions", "Search all actions", ""),
    ("settings", "Effective settings, sources and gates", ""),
    ("preferences", "Model, account and next-turn options", ""),
    (
        "attempts",
        "Read provider retry history",
        "Optional RUN_UUID and OFFSET; all OFFSET for all runs",
    ),
    (
        "inbox",
        "Read notification metadata (never responds)",
        "destinations | list DESTINATION [AFTER] | open/seen/dismiss DESTINATION EVENT",
    ),
    (
        "account",
        "Choose next-run account / private sign-in",
        "Optional safe label search",
    ),
    ("vessels", "Manage connected Vessels", ""),
    (
        "new",
        "Start a voyage",
        "Optional absolute workspace on the executing host",
    ),
    ("use", "Switch voyage", "Choose a voyage"),
    ("rename", "Rename this voyage", "Type a name"),
    (
        "branch",
        "Review a full or historical conversation branch",
        "Choose a saved user boundary in review; no file rollback",
    ),
    (
        "model",
        "Change the next-turn model",
        "Choose or type a model ID",
    ),
    (
        "thinking",
        "Change next-turn thinking",
        "Choose or type an effort; inherit clears",
    ),
    (
        "service",
        "Change next-turn service",
        "Choose or type a tier; inherit clears",
    ),
    ("models", "Show available models", ""),
    (
        "configure",
        "Load next-turn configuration",
        "Type an absolute configuration path on the executing host",
    ),
    (
        "inspect",
        "Inspect executing-host workspace",
        "Read-only status, staged/unstaged, untracked, file and directory scopes",
    ),
    (
        "diff",
        "Inspect staged/unstaged changes",
        "Executing host; no file rollback; binary markers only",
    ),
    (
        "copy",
        "Copy canonical assistant response",
        "Review local clipboard disclosure before Enter",
    ),
    ("tools", "Show available tools", ""),
    (
        "tool",
        "Run an operator tool",
        "Choose a tool, then type JSON arguments",
    ),
    ("policy", "Show permissions", ""),
    (
        "access",
        "Change voyage access",
        "Choose an access mode; Enter reviews the change",
    ),
    ("todos", "Show tasks", ""),
    ("subagents", "Show delegated work", ""),
    ("workflows", "Show saved workflows", ""),
    ("host_resources", "Show machine cleanup records", ""),
    (
        "browser",
        "View the Voyage host browser",
        "open | status | takeover | private | agent | close | detach",
    ),
    ("terminals", "Browse program terminals", ""),
    (
        "terminal",
        "Open a program terminal",
        "Choose a terminal or press Enter to browse",
    ),
    (
        "approve",
        "Allow a pending permission request",
        "Choose a pending approval",
    ),
    (
        "deny",
        "Deny a pending permission request",
        "Choose a pending approval",
    ),
    (
        "answer",
        "Answer a pending question",
        "Choose a question, then type your answer",
    ),
    (
        "stop",
        "Review Stop for the exact active run",
        "Enter reviews; detach does not stop",
    ),
    (
        "cancel",
        "Review Stop for the exact active run",
        "Alias for /stop",
    ),
    ("receipt", "Check an unconfirmed command", ""),
    (
        "compact",
        "Reduce older working context (not a generated summary)",
        "Keep newest N out of this reduction; older user text and canonical history remain",
    ),
    (
        "clear",
        "Clear this conversation",
        "Type this voyage's full UUID to confirm clearing history",
    ),
    (
        "archive",
        "Archive this voyage and release its process after cleanup",
        "",
    ),
    ("archived", "Browse archived voyages", ""),
    ("voyages", "Browse current voyages", ""),
    ("restore", "Restore this voyage", ""),
    (
        "delete",
        "Delete this conversation",
        "Type this voyage's full UUID to confirm deletion",
    ),
    (
        "export",
        "Save conversation as Markdown",
        "Type a destination path on this computer",
    ),
    ("conversation", "Return to the conversation", ""),
    ("quit", "Leave Helm; voyages keep running", ""),
];

/// Separate from every composer/private field, and never submitted to a provider.
#[derive(Default)]
pub(super) struct Discovery {
    pub query: String,
    pub settings: bool,
    pub detail_scroll: u16,
}

pub(super) fn shortcut(name: &str) -> &'static str {
    match name {
        "help" => "F1",
        "actions" => "F8",
        "new" => "Ctrl+N",
        "use" => "F2",
        "voyages" | "archived" => "F5 (toggle archive filter)",
        "terminals" | "terminal" => "F3",
        "browser" => "F6",
        "vessels" => "Ctrl+G",
        "quit" => "Ctrl+Q / Ctrl+C (not child terminal)",
        _ => "slash command",
    }
}
pub(super) fn scope(name: &str) -> &'static str {
    match name {
        "help" | "actions" | "settings" | "new" | "vessels" | "voyages" | "archived" | "quit" => {
            "Helm"
        }
        "account" | "model" | "thinking" | "service" | "configure" => "executing host · next run",
        "stop" | "cancel" => "exact active run",
        "approve" | "deny" | "answer" => "exact pending request",
        "browser" => "Voyage host browser + socket-bound control",
        _ => "selected voyage",
    }
}
pub(super) fn matches(query: &str) -> Vec<usize> {
    let words = query.to_lowercase();
    COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(index, (name, description, hint))| {
            let haystack = format!(
                "{name} {description} {hint} {} {}",
                scope(name),
                shortcut(name)
            )
            .to_lowercase();
            words
                .split_whitespace()
                .all(|word| haystack.contains(word))
                .then_some(index)
        })
        .collect()
}
fn opens_directly(name: &str) -> bool {
    matches!(
        name,
        "help"
            | "actions"
            | "preferences"
            | "settings"
            | "new"
            | "vessels"
            | "voyages"
            | "archived"
            | "account"
            | "model"
            | "thinking"
            | "service"
            | "access"
            | "tools"
            | "tool"
            | "todos"
            | "subagents"
            | "workflows"
            | "models"
            | "policy"
            | "host_resources"
            | "terminals"
            | "terminal"
            | "conversation"
            | "stop"
            | "cancel"
            | "inspect"
            | "diff"
            | "copy"
    )
}
impl App {
    /// An overlay only: never reads or dispatches the underlying private input.
    pub(super) fn discovery_input(&mut self, event: &crossterm::event::Event) -> bool {
        use crossterm::event::{Event, KeyCode, KeyEventKind};
        if self.terminal_request.is_some() {
            return false;
        }
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                return self.help;
            }
            if key.code == KeyCode::F(1) {
                self.discovery.settings = false;
                self.help = !self.help;
                self.help_scroll = 0;
                return true;
            }
            if self.help {
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('c' | 'q'))
                {
                    self.quit = true;
                    return true;
                }
                match key.code {
                    KeyCode::Esc => self.help = false,
                    KeyCode::PageUp | KeyCode::Up => {
                        self.help_scroll = self.help_scroll.saturating_sub(1)
                    }
                    KeyCode::PageDown | KeyCode::Down => {
                        self.help_scroll = self.help_scroll.saturating_add(1)
                    }
                    _ => {}
                }
                return true;
            }
        }
        self.help && !matches!(event, Event::Resize(_, _))
    }

    /// A reason is an observation, not an authority grant. Handlers revalidate on Enter.
    pub(super) fn discovery_reason(&self, name: &str) -> Option<String> {
        if matches!(
            name,
            "stop" | "cancel" | "receipt" | "approve" | "deny" | "answer"
        ) {
            let Some(view) = self.selected.and_then(|target| self.views.get(&target)) else {
                return Some("Select a voyage first (F2).".into());
            };
            match name {
                "stop" | "cancel" if !view.snapshot.as_ref().and_then(|s| s.run.as_ref()).is_some_and(|r| r.active()) =>
                    return Some("No observed active run to stop.".into()),
                "receipt" if view.pending.is_none() => return Some("No unresolved command receipt.".into()),
                "approve" | "deny" | "answer" if !view.snapshot.as_ref().is_some_and(|s| !s.decisions.is_empty()) =>
                    return Some("No observed pending request; use the request-specific controls when present.".into()),
                _ => {}
            }
        }
        if !opens_directly(name) {
            let (_, _, hint) = COMMANDS.iter().find(|(n, _, _)| *n == name)?;
            return Some(format!("Use /{name} explicitly in the composer. {hint}"));
        }
        if matches!(
            name,
            "help" | "actions" | "settings" | "new" | "vessels" | "voyages" | "archived"
        ) {
            return None;
        }
        if self.active_draft.is_some()
            && matches!(name, "account" | "model" | "thinking" | "service")
        {
            return None;
        }
        if self.active_draft.is_some() {
            return Some(
                "This action requires a voyage; the focused draft is not a session.".into(),
            );
        }
        let Some(target) = self.selected else {
            return Some("Select a voyage first (F2), or create a draft.".into());
        };
        let Some(view) = self.views.get(&target) else {
            return Some("Selected voyage is unavailable.".into());
        };
        if !self.clients.available(target.route) {
            return Some("Executing Vessel disconnected; reconnect first.".into());
        }
        if view.snapshot.is_none() {
            return Some("Waiting for an authenticated snapshot.".into());
        }
        None
    }
    pub(super) fn discovery_open(&mut self, name: &str) -> Result<()> {
        if let Some(reason) = self.discovery_reason(name) {
            self.status = reason;
            return Ok(());
        }
        self.explore = None;
        self.help = false;
        match name {
            "actions" => {
                self.discovery.query.clear();
                self.discovery.detail_scroll = 0;
                self.explore = Some(0);
            }
            "preferences" => self.open_model_options()?,
            "settings" => {
                self.discovery.settings = true;
                self.help = true;
                self.help_scroll = 0;
            }
            "help" => {
                self.discovery.settings = false;
                self.help = true;
                self.help_scroll = 0;
            }
            "new" => self.create(None)?,
            "inspect" | "diff" | "copy" => {
                if let Some(target) = self.selected {
                    self.command_for(target, format!("/{name}"), true)?;
                }
            }
            "stop" | "cancel" => {
                if let Some(target) = self.selected {
                    self.review_stop(target)?;
                }
            }
            "vessels" => self.open_vessels(),
            "voyages" | "archived" => {
                self.show_archives(name == "archived");
            }
            "account" | "model" | "thinking" | "service" => {
                let destination = self
                    .active_draft
                    .map(Destination::Draft)
                    .or(self.selected.map(Destination::Live));
                if let Some(destination) = destination {
                    self.inference_command(destination, &format!("/{name}"), true)?;
                }
            }
            _ => {
                if let Some(target) = self.selected {
                    match name {
                        "access" => self.open_access(target, None)?,
                        "tool" => self.open_operator(target, None)?,

                        "workflows" => self.open_workflows()?,
                        "terminals" | "terminal" => {
                            self.views
                                .get_mut(&target)
                                .expect("selected view")
                                .terminals
                                .open = true
                        }
                        "conversation" => {
                            self.views.get_mut(&target).expect("selected view").panel = None
                        }
                        section => self.inspect_control(target, section)?,
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn discovery_command(&mut self, command: &str) -> Result<bool> {
        match command {
            "/actions" | "/settings" | "/preferences" | "/help" => {
                self.discovery_open(&command[1..])?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    pub(super) fn discovery_help(&self) -> String {
        if self.discovery.settings {
            return self.discovery_settings();
        }
        let focus = if self
            .vessels
            .as_ref()
            .is_some_and(|manager| manager.borrow().is_open())
        {
            "Vessel connections: Esc backs out of the current step. Connection addresses and credentials belong only in this private form."
        } else if self.accounts.open() {
            "Accounts: private executing-host account selection/sign-in. Esc backs out; next-run changes do not rewrite the active run."
        } else if self.inspection.panel.is_some() || self.inspection.confirmation.is_some() {
            "Workspace inspection: executing-host runtime reads; no local fallback or rollback. Clipboard review: Enter confirms local canonical-text disclosure; Esc cancels."
        } else if self.stop_review.is_some() {
            "Stop review: Enter requests cancellation of the displayed exact run; Esc returns without stopping; Ctrl+C detaches and work continues."
        } else if self.workspace_picker.is_some() {
            "workspace picker: private path entry; Esc returns without changing the draft"
        } else if self.inference_picker_open() {
            "inference picker: next-turn settings; Esc cancels the selection"
        } else if self.workflows_open() {
            "workflow review: explicit workflow dispatch; Esc returns"
        } else if self.operator.is_some() {
            "operator tool form: explicit tool dispatch; Esc returns"
        } else if self.interactions.borrow().focused {
            "Needs you: use the visible request's controls. Esc is Deny/Skip only where the request labels say so; pending responses are checked automatically. Help does not answer the request."
        } else if self.voyage_picker.is_some() {
            "Voyage finder: type to filter, Up/Down choose, Enter opens the exact route-qualified voyage, Esc returns."
        } else if self.explore.is_some() {
            "Actions: type to filter names, descriptions, scopes or shortcuts. Up/Down choose, Enter opens an existing handler; unavailable rows explain why. Esc returns without changing any draft."
        } else if self.active_draft.is_some() {
            "New-voyage draft: first send creates the voyage; the draft itself is not a session. Model/account changes apply to that first run."
        } else {
            "Composer: Enter submits when idle and steers the active run. Tab completes a slash command, not a submission. Esc closes completion; it does not stop a run."
        };
        format!(
            "Help · current focus\n\n{focus}\n\nKeyboard fallback\nF8 or /actions opens searchable discovery. F1 or /help opens help; /settings shows effective settings. If function keys are intercepted, return to the composer with Esc and use the slash command. Never type commands into a password, address, question, or child-terminal field. Private fields retain their own input routing.\n\nPageUp/PageDown scroll help and selected action details even in a narrow terminal; Esc returns. In Actions, one row per result and a wrapped selected-action description keep details reachable without a mouse.\n\nLeaving Helm is not Stop\nCtrl+Q (outside modal/private focus) or /quit detaches Helm; voyages continue. /cancel requests cancellation of the selected active run, not observed cleanup. Ctrl+C in an attached private terminal belongs to the child; Ctrl+] returns to Helm.\n\nSearch /actions for the complete slash catalogue, scope, shortcuts and entry requirements.\n\n{}",
            extension_decision()
        ) + "\n\nKeyboard reference\n"
            + super::presentation::HELP
    }
    fn discovery_settings(&self) -> String {
        let mut text = String::from("Effective settings · read-only observations\n\n");
        if let Some(id) = self.active_draft {
            text.push_str(&format!("Draft {id}: not a running voyage. Account/model setup applies to its first run. Use the draft's private setup controls; no running settings are inferred from an older selected voyage.\n"));
        } else if let Some(target) = self.selected {
            text.push_str(&format!(
                "Voyage {} · Vessel connection {}\n",
                target.session, target.route.id
            ));
            if let Some(view) = self.views.get(&target) {
                text.push_str(if self.clients.available(target.route) {
                    "Source: last authenticated executing-host snapshot (not a new fetch).\n"
                } else {
                    "STALE: executing Vessel disconnected; these are last observed values.\n"
                });
                if let Some(snapshot) = &view.snapshot {
                    text.push_str(&format!("Revision {} · access: {}\nAccess source: executing voyage policy; not an OS sandbox. /access opens a reviewed change; routing cannot broaden authority.\n", snapshot.revision, snapshot.access.as_deref().map(super::safe).unwrap_or_else(|| "unknown".into())));
                    if let Some(current) = &snapshot.inference_current {
                        text.push_str(&settings_text("Current run (observed)", current));
                    }
                    if let Some(settings) = &snapshot.inference {
                        text.push_str(&settings_text(
                            if snapshot.inference_next_turn {
                                "Next run (active run unchanged)"
                            } else {
                                "Selected inference settings"
                            },
                            settings,
                        ));
                    } else {
                        text.push_str("Inference settings unavailable; no defaults invented.\n");
                    }
                    if snapshot.inference_current.is_none() {
                        text.push_str("Separate current-run inference observation unavailable.\n");
                    }
                } else {
                    text.push_str("Authenticated settings snapshot unavailable.\n");
                }
            }
        } else {
            text.push_str("No voyage selected. Use /new or F2.\n");
        }
        text.push_str("\nProvenance and gates\nProvider/model values above come from the executing host. Configuration-file path and override-layer provenance are not supplied by this snapshot; unknown is not 'default'. /configure loads an executing-host absolute path for the next run. Account selection/sign-in stays private on that host. Advertised model support is not account entitlement; provider-reported values are observations, not billing guarantees. /policy shows executing-host constraints; /tools shows the actual runtime registry, not a proposed capability list. Shared local browser execution requires separate Helm consent as well as voyage policy.\n\n");
        text.push_str(extension_decision());
        text
    }
}
fn settings_text(label: &str, settings: &super::inference::Settings) -> String {
    let mut text = format!(
        "\n{label}\nProvider: {} · model: {}\nThinking: {}\nService: {}\n",
        super::safe(&settings.provider),
        super::safe(&settings.model),
        super::safe(&settings.label(super::inference::Field::Thinking)),
        super::safe(&settings.label(super::inference::Field::Service))
    );
    if let Some(resolution) = &settings.resolution {
        for (name, value) in [
            ("Thinking", &resolution.thinking),
            ("Service", &resolution.service),
        ] {
            text.push_str(&format!("{name} source: {}; model support: {:?}; account support: {:?}; provider reported: {}\n", super::safe(&value.source), value.support, value.account_support, value.provider_reported.as_deref().map(super::safe).unwrap_or_else(|| "unknown".into())));
        }
    } else {
        text.push_str("Resolution/source/support metadata unavailable.\n");
    }
    text
}
fn extension_decision() -> &'static str {
    "Keymaps and extensions\nKeymaps are fixed in this release; no user-remapping or collision validator is implemented. No general Helm extension loader/extension trust catalogue is implemented. These are explicitly unsupported, not hidden configuration switches. Release proposal: retain fixed shortcuts and no general loader. Remapping without scope-aware collision validation can steal private input or make consent/escape unreachable; a loader without origin, permission and revocation contracts would blur the Helm/Voyage authority boundary. Workflows and the live tool registry already supply bounded alternatives, so this delivery should not add a partial loader or a cosmetic keymap setting. Future support requires scope/collision/focus validation and extension origin, trust, permissions and revocation design: tracked obligations https://github.com/o-psi/voyage/issues/272 (U12). Saved workflows are a separate trust/review surface (/workflows); runtime tools remain governed by the executing voyage."
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_includes_scope_shortcut_and_description() {
        assert!(
            matches("next run")
                .iter()
                .any(|i| COMMANDS[*i].0 == "account")
        );
        assert_eq!(
            matches("F8"),
            vec![COMMANDS.iter().position(|x| x.0 == "actions").unwrap()]
        );
        assert!(matches("not-a-real-action").is_empty());
    }
    #[test]
    fn names_unique_and_direct_actions_registered() {
        let names = COMMANDS
            .iter()
            .map(|x| x.0)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names.len(), COMMANDS.len());
        for name in ["actions", "settings", "help", "tools", "account", "access"] {
            assert!(names.contains(name));
            assert!(opens_directly(name));
        }
        for name in ["delete", "clear", "approve", "answer"] {
            assert!(
                !opens_directly(name),
                "destructive/request actions must remain explicit"
            );
        }
    }
    #[test]
    fn catalogue_commands_have_existing_dispatch_sources() {
        // Guard against adding attractive but nonexistent commands to discovery.
        let sources = concat!(
            include_str!("actions.rs"),
            include_str!("inference.rs"),
            include_str!("lifecycle.rs"),
            include_str!("inbox.rs"),
            include_str!("browser.rs"),
            include_str!("operator.rs"),
            include_str!("controls.rs")
        );
        for (name, _, _) in COMMANDS {
            if matches!(*name, "actions" | "settings" | "preferences" | "help") {
                continue;
            }
            assert!(
                sources.contains(&format!("/{name}")),
                "No dispatch source for /{name}"
            );
        }
    }

    #[test]
    fn unsupported_features_are_not_advertised_as_available() {
        assert!(extension_decision().contains("explicitly unsupported"));
        assert!(extension_decision().contains("issues/272"));
    }
}
