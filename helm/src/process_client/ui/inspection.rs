//! Read-only coding inspection. All workspace effects use the owner's ordinary
//! durable operator-tool path; this module never opens a workspace path locally.
use super::operator::{Observation, Request};
use anyhow::{Result, ensure};
use crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde_json::{Value, json};

const LIMIT: usize = 64 * 1024;
const GIT: &str = "git --no-pager --no-optional-locks -c core.quotePath=true -c color.ui=false -c core.fsmonitor=false";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Status,
    Unstaged,
    Staged,
    Untracked,
    Directory,
    File,
}
impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Status => "Changed paths (index/worktree status)",
            Self::Unstaged => "Unstaged: worktree versus index",
            Self::Staged => "Staged: index versus HEAD",
            Self::Untracked => "Untracked paths only (not diff contents)",
            Self::Directory => "Browse directory on executing host",
            Self::File => "Read file on executing host",
        }
    }
    pub fn request(self, observation: Observation, tools: &Value, path: &str) -> Result<Request> {
        // No shell interpolation of user paths, including Git pathspec magic.
        let path = relative_path(path)?;
        let (name, arguments) = match self {
            Self::Directory => ("list_directory", json!({"path":path,"recursive":false})),
            Self::File => ("read_file", json!({"path":path})),
            scope => {
                let suffix = match scope {
                    Self::Status => "status --porcelain=v1 --untracked-files=normal",
                    Self::Untracked => "ls-files --others --exclude-standard",
                    Self::Unstaged => {
                        "diff --no-ext-diff --no-textconv --no-color --no-renames --submodule=short"
                    }
                    Self::Staged => {
                        "diff --cached --no-ext-diff --no-textconv --no-color --no-renames --submodule=short"
                    }
                    _ => unreachable!(),
                };
                let command = format!("{GIT} {suffix} --");
                ("shell", json!({"command":command}))
            }
        };
        let value = inventory(tools);
        ensure!(
            value.as_array().is_some_and(|entries| entries
                .iter()
                .any(|t| t["name"] == name && t["input_schema"].is_object())),
            "Executing voyage does not advertise {name}; inspection unavailable (no local fallback)"
        );
        Ok(Request {
            observation,
            name: name.into(),
            arguments,
        })
    }
}

/// Idle preflight wraps the same definitions with honest readiness metadata.
/// Active owners may return the inventory array directly; neither is authority.
pub(super) fn inventory(tools: &Value) -> &Value {
    let value = tools.get("value").unwrap_or(tools);
    value.get("inventory").unwrap_or(value)
}

fn relative_path(path: &str) -> Result<String> {
    let path = if path.is_empty() { "." } else { path };
    ensure!(
        path.len() <= 4096 && !path.chars().any(char::is_control),
        "Path is too long or contains control characters"
    );
    ensure!(
        !path.starts_with('/')
            && !path.contains('\\')
            && !path.contains(':')
            && !path.split('/').any(|p| p == ".."),
        "Use an executing-workspace-relative path without parent traversal"
    );
    Ok(path.into())
}

pub(super) enum Outcome {
    Stay,
    Close,
    Submit(Request),
    CopyResponse,
}

pub(super) struct Panel {
    pub observation: Observation,
    pub context: String,
    tools: Value,
    selection: usize,
    path: String,
    editing: bool,
    body: String,
    scroll: u16,
    error: String,
}
const SCOPES: [Scope; 6] = [
    Scope::Status,
    Scope::Unstaged,
    Scope::Staged,
    Scope::Untracked,
    Scope::Directory,
    Scope::File,
];
impl Panel {
    pub fn new(observation: Observation, context: String, tools: Value) -> Self {
        Self {
            observation,
            context,
            tools,
            selection: 0,
            path: ".".into(),
            editing: false,
            body: String::new(),
            scroll: 0,
            error: String::new(),
        }
    }
    pub fn set_error(&mut self, error: String) {
        self.error = bounded(&error);
    }
    /// Only pass the result of the exact admitted request, never the latest tool
    /// indiscriminately. A refusal/timeout is shown as such, not a clean diff.
    pub fn result(&mut self, text: &str) {
        self.body = bounded(text);
        self.scroll = 0;
    }
    pub fn input(&mut self, event: &Event) -> Outcome {
        let Event::Key(key) = event else {
            return Outcome::Stay;
        };
        if key.kind != KeyEventKind::Press {
            return Outcome::Stay;
        }
        if self.editing {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => self.editing = false,
                KeyCode::Backspace => {
                    self.path.pop();
                }
                KeyCode::Char(ch)
                    if !ch.is_control() && self.path.len() + ch.len_utf8() <= 4096 =>
                {
                    self.path.push(ch)
                }
                _ => {}
            }
            return Outcome::Stay;
        }
        match key.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Up => self.selection = self.selection.saturating_sub(1),
            KeyCode::Down => self.selection = (self.selection + 1).min(SCOPES.len() - 1),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::Char('p') => self.editing = true,
            KeyCode::Char('c') => return Outcome::CopyResponse,
            KeyCode::Enter => {
                match SCOPES[self.selection].request(self.observation, &self.tools, &self.path) {
                    Ok(request) => return Outcome::Submit(request),
                    Err(error) => self.set_error(error.to_string()),
                }
            }
            _ => {}
        }
        Outcome::Stay
    }
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let mut lines = vec![self.context.clone(),
            "Read-only observation, not rollback. Whole executing workspace Git scope; unrelated/pre-existing edits included.".into(),
            "Staged and unstaged are separate. Untracked contents/ignored files excluded from diffs. Binary: marker only; no binary patch. Submodules: short summary.".into(),
            "Git required for diffs. Runtime policy/approvals and output limits apply; refusal or missing output is not a clean tree. Non-UTF paths may be quoted; file text may be lossy. No local fallback.".into(),
            "↑/↓ choose · Enter inspect once · p edit relative path · c copy canonical response · PgUp/PgDn scroll · Esc return".into(),
            format!("Path (file/directory only){}: {}", if self.editing { " [editing]" } else { "" }, self.path)];
        for (i, scope) in SCOPES.iter().enumerate() {
            lines.push(format!(
                "{} {}",
                if i == self.selection { "›" } else { " " },
                scope.label()
            ));
        }
        if !self.error.is_empty() {
            lines.push(format!("Unavailable: {}", self.error));
        }
        lines.push(self.body.clone());
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(crate::process_client::safe(&lines.join("\n")))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Inspect coding results · executing host"),
                )
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0)),
            area,
        );
    }
}
fn bounded(text: &str) -> String {
    if text.len() <= LIMIT {
        return text.into();
    }
    let mut end = LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[Inspection display truncated at 64 KiB; omitted output is unknown]",
        &text[..end]
    )
}

#[cfg(test)]
mod tests {
    use super::super::state::{Route, Target};
    use super::*;
    use uuid::Uuid;
    fn observation() -> Observation {
        Observation {
            target: Target {
                route: Route {
                    id: Uuid::new_v4(),
                    generation: 3,
                },
                session: Uuid::new_v4(),
            },
            incarnation: Uuid::new_v4(),
            revision: 4,
            run_id: None,
        }
    }
    fn tools() -> Value {
        json!({"value":[{"name":"shell","input_schema":{}},{"name":"read_file","input_schema":{}},{"name":"list_directory","input_schema":{}}]})
    }
    #[test]
    fn idle_preflight_inventory_is_supported_without_claiming_execution_readiness() {
        let direct = tools();
        let wrapped = json!({"section":"tools","value":{"inventory":direct["value"],"source":"builtin_preflight"}});
        assert_eq!(inventory(&direct), inventory(&wrapped));
        assert!(!inventory(&json!({"section":"tools","value":{}})).is_array());
    }
    #[test]
    fn remote_request_retains_exact_target_and_never_interpolates_paths() {
        let o = observation();
        let r = Scope::Unstaged
            .request(o, &tools(), "$(touch injected)")
            .unwrap();
        assert_eq!(r.observation, o);
        assert!(!r.arguments["command"].as_str().unwrap().contains("touch"));
        assert!(
            r.arguments["command"]
                .as_str()
                .unwrap()
                .contains("--no-ext-diff --no-textconv")
        );
        assert_eq!(
            Scope::File
                .request(o, &tools(), "a file")
                .unwrap()
                .arguments,
            json!({"path":"a file"})
        );
    }
    #[test]
    fn refused_registry_and_unsafe_paths_do_not_fallback() {
        assert!(
            Scope::Unstaged
                .request(observation(), &json!([]), ".")
                .is_err()
        );
        for path in ["../a", "/tmp/a", "a/../b", "C:\\a", "a\u{0}b"] {
            assert!(relative_path(path).is_err());
        }
    }
    #[test]
    fn display_is_bounded_at_unicode_boundary_and_sanitized() {
        let s = bounded(&"é".repeat(LIMIT));
        assert!(s.len() < LIMIT + 100);
        assert!(s.contains("truncated"));
        assert!(!crate::process_client::safe("\x1b[31m\u{202e}bad").contains('\x1b'));
    }
    #[test]
    #[cfg(unix)]
    fn ordinary_dirty_tree_scopes_binary_and_absent_git() {
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(output.status.success(), "fixture Git command failed");
        };
        git(&["init", "-q"]);
        std::fs::write(dir.path().join("tracked"), "base\n").unwrap();
        std::fs::write(dir.path().join("binary"), [0, 1, 2]).unwrap();
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "base",
        ]);
        std::fs::write(dir.path().join("tracked"), "staged\n").unwrap();
        git(&["add", "tracked"]);
        std::fs::write(dir.path().join("tracked"), "unstaged\n").unwrap();
        std::fs::write(dir.path().join("binary"), [0, 3, 4]).unwrap();
        std::fs::write(dir.path().join("new file"), "untracked\n").unwrap();
        let run = |scope: Scope| {
            let request = scope.request(observation(), &tools(), ".").unwrap();
            let output = Command::new("sh")
                .args(["-c", request.arguments["command"].as_str().unwrap()])
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap()
        };
        assert!(run(Scope::Status).contains("MM tracked"));
        assert!(run(Scope::Staged).contains("+staged"));
        let unstaged = run(Scope::Unstaged);
        assert!(unstaged.contains("+unstaged"));
        assert!(unstaged.contains("Binary files"));
        assert!(!unstaged.contains("GIT binary patch"));
        assert!(!unstaged.contains("new file"));
        assert!(run(Scope::Untracked).contains("new file"));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let path = std::ffi::OsString::from_vec(vec![b'n', 0xff]);
            std::fs::write(dir.path().join(path), "non-UTF path").unwrap();
            assert!(run(Scope::Untracked).contains("\\377"));
        }
        let request = Scope::Staged.request(observation(), &tools(), ".").unwrap();
        let missing = Command::new("/bin/sh")
            .args(["-c", request.arguments["command"].as_str().unwrap()])
            .env("PATH", dir.path().join("absent-bin"))
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(!missing.status.success());
        assert!(missing.stdout.is_empty());
    }
}
