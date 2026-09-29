//! Bounded executing-host filename discovery for a human composer draft.
//! Names are observations, never authorization to read a file later.
use crate::policy::Policy;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::VecDeque;

const MAX_SCANNED: usize = 8192;
const MAX_FILES: usize = 1024;
const MAX_DEPTH: usize = 8;
const MAX_PATH_BYTES: usize = 4096;
const EXCLUDED: &[&str] = &[".git", "node_modules", "target", "vendor", ".venv"];

pub(super) fn read(policy: &Policy) -> Result<Value> {
    policy.check_current()?;
    let root = policy.resolve_read(policy.workspace())?;
    ensure!(root.is_dir(), "workspace unavailable");
    let mut queue = VecDeque::from([(root.clone(), 0usize)]);
    let mut paths = Vec::new();
    let mut scanned = 0usize;
    let mut truncated = false;
    while let Some((directory, depth)) = queue.pop_front() {
        policy.check_current()?;
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                truncated = true;
                continue;
            }
        };
        for entry in entries {
            scanned += 1;
            if scanned > MAX_SCANNED {
                truncated = true;
                break;
            }
            let Ok(entry) = entry else {
                truncated = true;
                continue;
            };
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                truncated = true;
                continue;
            };
            if EXCLUDED.contains(&name.as_str()) {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                truncated = true;
                continue;
            };
            if kind.is_symlink() {
                truncated = true;
                continue;
            }
            let path = entry.path();
            let Ok(resolved) = policy.resolve_read(&path) else {
                truncated = true;
                continue;
            };
            if !resolved.starts_with(&root) {
                truncated = true;
                continue;
            }
            if kind.is_dir() {
                if depth < MAX_DEPTH {
                    queue.push_back((path, depth + 1));
                } else {
                    truncated = true;
                }
                continue;
            }
            if !kind.is_file() {
                truncated = true;
                continue;
            }
            let relative = path.strip_prefix(&root)?;
            let Some(relative) = relative.to_str() else {
                truncated = true;
                continue;
            };
            if relative.len() > MAX_PATH_BYTES || relative.chars().any(char::is_control) {
                truncated = true;
                continue;
            }
            if paths.len() >= MAX_FILES {
                truncated = true;
                break;
            }
            paths.push(relative.to_owned());
        }
        if truncated && (scanned > MAX_SCANNED || paths.len() >= MAX_FILES) {
            break;
        }
    }
    paths.sort_unstable();
    policy.check_current()?;
    Ok(json!({
        "files": paths,
        "truncated": truncated,
        "excluded_directories": EXCLUDED,
        "observed_at_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn catalogue_lists_only_allowed_workspace_names_and_reports_limits() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(workspace.join("src")).unwrap();
        fs::write(workspace.join("src/main.rs"), "content remains private").unwrap();
        fs::write(root.path().join("outside.txt"), "outside").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            root.path().join("outside.txt"),
            workspace.join("linked.txt"),
        )
        .unwrap();
        fs::create_dir(workspace.join("node_modules")).unwrap();
        fs::write(workspace.join("node_modules/hidden.js"), "dependency").unwrap();
        let policy = Policy::new(&crate::Config::default(), workspace.clone()).unwrap();
        let listing = read(&policy).unwrap();
        assert_eq!(listing["files"], json!(["src/main.rs"]));
        assert_eq!(listing["truncated"], cfg!(unix));
        for index in 0..=MAX_FILES {
            fs::write(workspace.join(format!("file-{index:04}.txt")), "x").unwrap();
        }
        let limited = read(&policy).unwrap();
        assert_eq!(limited["truncated"], true);
        assert_eq!(limited["files"].as_array().unwrap().len(), MAX_FILES);
    }
}
