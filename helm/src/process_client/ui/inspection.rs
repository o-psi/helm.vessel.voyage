//! Read-only coding inspection. All workspace effects use the owner's ordinary
//! durable operator-tool path; this module never opens a workspace path locally.
use super::operator::{Observation, Request};
use anyhow::{Result, ensure};
use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde_json::{Value, json};

const LIMIT: usize = 64 * 1024;
const GIT: &str = "git --literal-pathspecs --no-pager --no-optional-locks -c core.quotePath=true -c color.ui=false -c core.fsmonitor=false";

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
                    Self::Status => "status --porcelain=v1 --no-renames --untracked-files=all",
                    Self::Untracked => "ls-files --others --exclude-standard",
                    Self::Unstaged => {
                        "diff --no-ext-diff --no-textconv --no-color --no-renames --submodule=short"
                    }
                    Self::Staged => {
                        "diff --cached --no-ext-diff --no-textconv --no-color --no-renames --submodule=short"
                    }
                    _ => unreachable!(),
                };
                let command = format!("{GIT} {suffix} -- {}", shell_path(&path));
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

fn shell_path(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\"'\"'"))
}
fn git_path(text: &str) -> Option<String> {
    if !text.starts_with('"') {
        return relative_path(text).ok();
    }
    let text = text.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = Vec::new();
    let mut bytes = text.bytes().peekable();
    while let Some(b) = bytes.next() {
        if b != b'\\' {
            out.push(b);
            continue;
        }
        match bytes.next()? {
            b'"' => out.push(b'"'),
            b'\\' => out.push(b'\\'),
            b @ b'0'..=b'7' => {
                let mut n = (b - b'0') as u16;
                for _ in 0..2 {
                    let b = bytes.next()?;
                    if !(b'0'..=b'7').contains(&b) {
                        return None;
                    }
                    n = n * 8 + (b - b'0') as u16;
                }
                out.push(u8::try_from(n).ok()?);
            }
            _ => return None,
        }
    }
    relative_path(&String::from_utf8(out).ok()?).ok()
}
fn changed_paths(text: &str) -> Vec<String> {
    // Accept only a successful shell result from the admitted status invocation.
    // No arbitrary assistant prose or truncated/failure output becomes navigation.
    let Some((_, rest)) = text.split_once("exit: 0\nstdout:\n") else {
        return Vec::new();
    };
    let Some((stdout, _)) = rest.split_once("\nstderr:") else {
        return Vec::new();
    };
    if stdout.len() > LIMIT {
        return Vec::new();
    }
    stdout
        .lines()
        .filter_map(|line| {
            let b = line.as_bytes();
            if b.len() < 4 || b[2] != b' ' || !b[..2].iter().all(|b| b" MADRCU?!".contains(b)) {
                return None;
            }
            git_path(&line[3..])
        })
        .take(256)
        .collect()
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
    result_scope: Option<Scope>,
    paths: Vec<String>,
    file_selection: Option<usize>,
    file_hits: std::cell::RefCell<Vec<(Rect, usize)>>,
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
            result_scope: None,
            paths: Vec::new(),
            file_selection: None,
            file_hits: Default::default(),
        }
    }
    pub fn set_error(&mut self, error: String) {
        self.error = bounded(&error);
        self.file_hits.borrow_mut().clear();
    }
    /// Only pass the result of the exact admitted request, never the latest tool
    /// indiscriminately. A refusal/timeout is shown as such, not a clean diff.
    pub fn result(&mut self, text: &str) {
        self.body = bounded(text);
        self.file_hits.borrow_mut().clear();
        if self.result_scope == Some(Scope::Status) {
            self.paths = changed_paths(text);
            self.file_selection = None;
        }
        self.scroll = 0;
    }
    pub fn input(&mut self, event: &Event) -> Outcome {
        if matches!(event, Event::Resize(..)) {
            self.file_hits.borrow_mut().clear();
            return Outcome::Stay;
        }
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            let selected = self
                .file_hits
                .borrow()
                .iter()
                .find(|(r, _)| r.contains((mouse.column, mouse.row).into()))
                .map(|(_, i)| *i);
            if let Some(i) = selected {
                self.file_selection = Some(i);
                self.path = self.paths[i].clone();
                return self.read(Scope::File);
            }
            return Outcome::Stay;
        }
        let Event::Key(key) = event else {
            return Outcome::Stay;
        };
        if key.kind != KeyEventKind::Press {
            return Outcome::Stay;
        }
        self.file_hits.borrow_mut().clear();
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
            KeyCode::Tab if !self.paths.is_empty() => {
                self.file_selection = if self.file_selection.is_some() {
                    None
                } else {
                    Some(0)
                };
            }
            KeyCode::Up if self.file_selection.is_some() => {
                self.file_selection = self.file_selection.map(|i| i.saturating_sub(1))
            }
            KeyCode::Down if self.file_selection.is_some() => {
                self.file_selection = self
                    .file_selection
                    .map(|i| (i + 1).min(self.paths.len() - 1))
            }
            KeyCode::Up => self.selection = self.selection.saturating_sub(1),
            KeyCode::Down => self.selection = (self.selection + 1).min(SCOPES.len() - 1),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::Char('p') => self.editing = true,
            KeyCode::Char('c') => return Outcome::CopyResponse,
            KeyCode::Char('a') => {
                self.path = ".".into();
                self.file_selection = None;
                return self.read(Scope::Status);
            }
            KeyCode::Char('u') if self.file_selection.is_some() => {
                self.path = self.paths[self.file_selection.unwrap()].clone();
                return self.read(Scope::Unstaged);
            }
            KeyCode::Char('s') if self.file_selection.is_some() => {
                self.path = self.paths[self.file_selection.unwrap()].clone();
                return self.read(Scope::Staged);
            }
            KeyCode::Enter if self.file_selection.is_some() => {
                self.path = self.paths[self.file_selection.unwrap()].clone();
                return self.read(Scope::File);
            }
            KeyCode::Enter => return self.read(SCOPES[self.selection]),
            _ => {}
        }
        Outcome::Stay
    }
    fn read(&mut self, scope: Scope) -> Outcome {
        match scope.request(self.observation, &self.tools, &self.path) {
            Ok(request) => {
                self.error.clear();
                self.result_scope = Some(scope);
                Outcome::Submit(request)
            }
            Err(error) => {
                self.error = error.to_string();
                Outcome::Stay
            }
        }
    }
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        self.file_hits.borrow_mut().clear();
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Changes and files · executing host ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height < 10 {
            return;
        }
        let title = format!(
            "{}\nRead-only · includes pre-existing edits · no rollback\n{}\nPath: {}\n↑↓ scope · Enter read · p path · a all · Tab files · Esc back",
            self.context,
            self.result_scope.unwrap_or(SCOPES[self.selection]).label(),
            self.path
        );
        frame.render_widget(
            Paragraph::new(title).wrap(Wrap { trim: false }),
            Rect::new(inner.x, inner.y, inner.width, 5),
        );
        let n = if self.paths.is_empty() {
            0
        } else {
            (inner.height.saturating_sub(9) as usize).min(6)
        };
        let first = self
            .file_selection
            .unwrap_or(0)
            .saturating_sub(n.saturating_sub(1));
        for (i, path) in self.paths.iter().enumerate().skip(first).take(n) {
            let rect = Rect::new(inner.x, inner.y + 5 + (i - first) as u16, inner.width, 1);
            frame.render_widget(
                Paragraph::new(format!(
                    "{} {}",
                    if self.file_selection == Some(i) {
                        ">"
                    } else {
                        " "
                    },
                    path
                ))
                .style(if self.file_selection == Some(i) {
                    crate::theme::Role::Selection.style()
                } else {
                    crate::theme::Role::Focus.style()
                }),
                rect,
            );
            self.file_hits.borrow_mut().push((rect, i));
        }
        let top = inner.y + 5 + n as u16;
        let hint = if self.paths.is_empty() {
            "Binary diffs: markers only · untracked contents require file read"
        } else {
            "Select file: Enter/click read · u unstaged · s staged · a all"
        };
        frame.render_widget(
            Paragraph::new(hint),
            Rect::new(inner.x, top, inner.width, 1),
        );
        let text = if self.error.is_empty() {
            &self.body
        } else {
            &self.error
        };
        frame.render_widget(
            Paragraph::new(crate::process_client::safe(text))
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0)),
            Rect::new(
                inner.x,
                top + 1,
                inner.width,
                inner.bottom().saturating_sub(top + 1),
            ),
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
    fn porcelain_paths_are_bounded_and_success_only() {
        let text = "Exact admitted operator run fixture · completed\nexit: 0\nstdout:\n M plain.txt\n?? a file.txt\n M \"caf\\303\\251.txt\"\n M ../escape\n M \"bad\\nname\"\nstderr:\n";
        assert_eq!(
            changed_paths(text),
            vec!["plain.txt", "a file.txt", "café.txt"]
        );
        assert!(changed_paths(&text.replace("exit: 0", "exit: 1")).is_empty());
        assert!(changed_paths(" M not-a-tool-result").is_empty());
        assert_eq!(shell_path("a'b"), "'a'\"'\"'b'");
    }
    #[test]
    fn selected_file_read_and_diff_use_exact_owner_and_literal_path() {
        let mut p = Panel::new(observation(), "fixture".into(), tools());
        assert!(matches!(
            p.input(&Event::Key(crossterm::event::KeyEvent::new(
                KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE
            ))),
            Outcome::Submit(_)
        ));
        p.result("exit: 0\nstdout:\n M a file.txt\nstderr:\n");
        p.input(&Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        )));
        let Outcome::Submit(r) = p.input(&Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ))) else {
            panic!("file must submit read")
        };
        assert_eq!(r.name, "read_file");
        assert_eq!(r.arguments, json!({"path":"a file.txt"}));
        assert_eq!(r.observation, p.observation);
    }
    #[test]
    fn file_click_uses_rendered_path_and_resize_invalidates_hits() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut p = Panel::new(observation(), "fixture".into(), tools());
        p.result_scope = Some(Scope::Status);
        p.result("exit: 0\nstdout:\n M file.txt\nstderr:\n");
        let mut t = Terminal::new(TestBackend::new(40, 18)).unwrap();
        t.draw(|f| p.render(f, f.area())).unwrap();
        let (r, _) = p.file_hits.borrow()[0];
        let click = Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: r.x,
            row: r.y,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        let Outcome::Submit(request) = p.input(&click) else {
            panic!("click should read selected file")
        };
        assert_eq!(request.arguments, json!({"path":"file.txt"}));
        p.input(&Event::Resize(41, 18));
        assert!(matches!(p.input(&click), Outcome::Stay));
    }
    #[test]
    #[cfg(unix)]
    fn literal_git_path_cannot_inject_shell_or_pathspec() {
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success()
        );
        let path = "$(touch injected)' * [abc]";
        let request = Scope::Untracked
            .request(observation(), &tools(), path)
            .unwrap();
        assert!(
            Command::new("sh")
                .args(["-c", request.arguments["command"].as_str().unwrap()])
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success()
        );
        assert!(!dir.path().join("injected").exists());
        assert!(relative_path(":(glob)**").is_err());
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
        assert!(
            r.arguments["command"]
                .as_str()
                .unwrap()
                .ends_with("-- '$(touch injected)'")
        );
        assert!(
            r.arguments["command"]
                .as_str()
                .unwrap()
                .contains("--literal-pathspecs")
        );
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
