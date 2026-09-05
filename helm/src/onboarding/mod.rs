//! Deterministic project discovery. Repository content is evidence, never executable setup.
mod storage;
use crate::{Config, config::AccessMode};
use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX_FILE: usize = 256 * 1024;
const MAX_ENTRIES: usize = 512;
const MAX_PROJECTS: usize = 64;
const GUIDANCE: &[&str] = &[
    "AGENTS.md",
    "agents.md",
    "README.md",
    "README",
    "CONTRIBUTING.md",
];
const MANIFESTS: &[&str] = &["Cargo.toml", "package.json", "pyproject.toml", "go.mod"];

#[derive(Args)]
pub struct OnboardArgs {
    #[command(subcommand)]
    pub command: OnboardCommand,
}
#[derive(Subcommand)]
pub enum OnboardCommand {
    /// Inspect bounded project evidence without executing commands or loading a provider.
    Inspect {
        #[arg(long)]
        json: bool,
    },
    /// Generate a reviewable Markdown draft, optionally diffing an earlier draft.
    Preview {
        #[arg(long, conflicts_with = "output")]
        against: Option<PathBuf>,
        /// Create a new draft file; existing destinations are never replaced.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Explicit local confirmation for creating the requested draft file.
        #[arg(long, requires = "output")]
        confirm: bool,
    },
    /// Publish reviewed UTF-8 guidance to a new file, preserving existing instructions.
    Accept {
        #[arg(long)]
        draft: PathBuf,
        /// SHA-256 of the exact reviewed (possibly edited) draft; required confirmation.
        #[arg(long)]
        sha256: String,
        #[arg(long)]
        output: PathBuf,
    },
}
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub path: String,
    pub sha256: String,
    pub kind: String,
}
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub argv: Vec<String>,
    pub evidence: String,
    pub confidence: String,
    pub verified: bool,
}
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Project {
    pub directory: String,
    pub ecosystem: String,
    pub commands: Vec<Candidate>,
}
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Report {
    pub schema: u32,
    pub evidence: Vec<Evidence>,
    pub projects: Vec<Project>,
    pub warnings: Vec<String>,
}
fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}
fn candidate(args: &[&str], evidence: &str, confidence: &str) -> Candidate {
    Candidate {
        argv: args.iter().map(|s| (*s).into()).collect(),
        evidence: evidence.into(),
        confidence: confidence.into(),
        verified: false,
    }
}
fn parse_project(name: &str, text: &str, source: &str, directory: &str) -> Result<Project> {
    let (ecosystem, commands) = match name {
        "Cargo.toml" => {
            let value: toml::Value =
                toml::from_str(text).map_err(|_| anyhow::anyhow!("malformed Rust manifest"))?;
            if value.get("package").is_none() && value.get("workspace").is_none() {
                bail!("Rust manifest lacks package/workspace")
            }
            (
                "Rust",
                vec![
                    candidate(&["cargo", "build"], source, "manifest"),
                    candidate(&["cargo", "test"], source, "manifest"),
                    candidate(
                        &["cargo", "fmt", "--all", "--", "--check"],
                        source,
                        "inferred",
                    ),
                ],
            )
        }
        "package.json" => {
            let value: serde_json::Value = serde_json::from_str(text)
                .map_err(|_| anyhow::anyhow!("malformed JavaScript manifest"))?;
            if !value.is_object() {
                bail!("JavaScript manifest must be an object")
            }
            let runner = value
                .get("packageManager")
                .and_then(|v| v.as_str())
                .and_then(|v| v.split('@').next())
                .filter(|v| matches!(*v, "npm" | "pnpm" | "yarn" | "bun"))
                .unwrap_or("npm");
            let mut commands = Vec::new();
            if let Some(scripts) = value.get("scripts") {
                let scripts = scripts
                    .as_object()
                    .ok_or_else(|| anyhow::anyhow!("scripts must be an object"))?;
                for name in ["build", "test", "lint", "typecheck"] {
                    if scripts.get(name).is_some_and(|v| v.is_string()) {
                        commands.push(candidate(
                            &[runner, "run", name],
                            source,
                            "declared script; inspect its body before execution",
                        ));
                    }
                }
            }
            ("JavaScript/TypeScript", commands)
        }
        "pyproject.toml" => {
            let value: toml::Value =
                toml::from_str(text).map_err(|_| anyhow::anyhow!("malformed Python manifest"))?;
            if value.get("project").is_none()
                && value.get("build-system").is_none()
                && value.get("tool").is_none()
            {
                bail!("Python manifest has no recognized project metadata")
            }
            let mut commands = Vec::new();
            if value.get("tool").and_then(|v| v.get("pytest")).is_some() {
                commands.push(candidate(
                    &["python3", "-m", "pytest"],
                    source,
                    "configured tool",
                ));
            }
            if value.get("tool").and_then(|v| v.get("ruff")).is_some() {
                commands.push(candidate(
                    &["python3", "-m", "ruff", "check", "."],
                    source,
                    "configured tool",
                ));
            }
            ("Python", commands)
        }
        "go.mod" => {
            if !text.lines().any(|l| l.trim().starts_with("module ")) {
                bail!("Go manifest lacks module declaration")
            }
            (
                "Go",
                vec![
                    candidate(&["go", "build", "./..."], source, "manifest"),
                    candidate(&["go", "test", "./..."], source, "manifest"),
                ],
            )
        }
        _ => unreachable!(),
    };
    Ok(Project {
        directory: directory.into(),
        ecosystem: ecosystem.into(),
        commands,
    })
}
pub fn inspect(workspace: &Path) -> Result<Report> {
    let storage = storage::Root::open(workspace)?;
    let mut report = Report {
        schema: 1,
        evidence: vec![],
        projects: vec![],
        warnings: vec![],
    };
    let mut directories = vec![(PathBuf::new(), 0)];
    let mut seen = 0;
    while let Some((directory, depth)) = directories.pop() {
        let mut children = Vec::new();
        for entry in storage.entries(&directory)? {
            seen += 1;
            if seen > MAX_ENTRIES {
                bail!("onboarding discovery exceeds 512 entries; select a narrower workspace")
            }
            if !safe_name(&entry.name) {
                report
                    .warnings
                    .push("Skipped a nonportable or unsafe filename".into());
                continue;
            }
            if entry.directory
                && depth < 2
                && !matches!(
                    entry.name.as_str(),
                    "target"
                        | "node_modules"
                        | "dist"
                        | "vendor"
                        | ".git"
                        | ".local-git"
                        | ".venv"
                        | "venv"
                        | "__pycache__"
                )
                && !entry.name.starts_with('.')
            {
                children.push((directory.join(entry.name), depth + 1));
                continue;
            }
            if !MANIFESTS.contains(&entry.name.as_str()) && !GUIDANCE.contains(&entry.name.as_str())
            {
                continue;
            }
            let relative = directory.join(&entry.name);
            let source = relative.to_string_lossy().replace('\\', "/");
            let bytes = match storage.read(&relative, MAX_FILE) {
                Ok(bytes) => bytes,
                Err(_) => {
                    report.warnings.push(format!(
                        "Could not safely read {source}; inspect it manually"
                    ));
                    continue;
                }
            };
            let kind = if GUIDANCE.contains(&entry.name.as_str()) {
                "existing guidance (takes precedence)"
            } else {
                "manifest"
            };
            report.evidence.push(Evidence {
                path: source.clone(),
                sha256: digest(&bytes),
                kind: kind.into(),
            });
            if kind == "manifest" {
                if report.projects.len() >= MAX_PROJECTS {
                    bail!("too many projects; select a narrower workspace")
                }
                match std::str::from_utf8(&bytes).ok().and_then(|text| {
                    parse_project(
                        &entry.name,
                        text,
                        &source,
                        &if directory.as_os_str().is_empty() {
                            ".".into()
                        } else {
                            directory.to_string_lossy().replace('\\', "/")
                        },
                    )
                    .ok()
                }) {
                    Some(project) => report.projects.push(project),
                    None => report.warnings.push(format!(
                        "Malformed or unsupported {source}; no commands inferred"
                    )),
                }
            }
        }
        children.sort();
        children.reverse();
        directories.extend(children);
    }
    report.evidence.sort_by(|a, b| a.path.cmp(&b.path));
    report
        .projects
        .sort_by(|a, b| (&a.directory, &a.ecosystem).cmp(&(&b.directory, &b.ecosystem)));
    report.warnings.sort();
    report.warnings.dedup();
    if report.projects.is_empty() {
        report
            .warnings
            .push("No supported manifest found; consult existing project guidance".into());
    }
    Ok(report)
}
pub fn render(report: &Report) -> String {
    let mut out = String::from(
        "# Project guidance draft\n\nReview before use. Existing project instructions take precedence. This draft does not change Helm roots, tools, approvals or access policy. No command below has been executed or verified. Inspect package scripts before running them.\n\nDiscovery examines at most 512 entries and two directory levels. Deeper projects and files omitted from this report still may contain relevant instructions. Repository prose and script bodies are not copied into this draft.\n\n## Evidence\n\n",
    );
    for e in &report.evidence {
        out.push_str(&format!(
            "- `{}` — {}; SHA-256 `{}`\n",
            e.path, e.kind, e.sha256
        ));
    }
    out.push_str("\n## Candidate commands (unverified)\n\n");
    for p in &report.projects {
        out.push_str(&format!("### {} in `{}`\n\n", p.ecosystem, p.directory));
        if p.commands.is_empty() {
            out.push_str("No command inferred; consult project documentation.\n\n");
        }
        for c in &p.commands {
            out.push_str(&format!(
                "- `{}` — evidence `{}`; confidence: {}; **unverified**.\n",
                c.argv.join(" "),
                c.evidence,
                c.confidence
            ));
        }
        out.push('\n');
    }
    if !report.warnings.is_empty() {
        out.push_str("## Review required\n\n");
        for warning in &report.warnings {
            out.push_str(&format!("- {warning}\n"));
        }
    }
    out
}
pub fn run(args: OnboardArgs, config: &Config, workspace: Option<PathBuf>) -> Result<()> {
    let workspace = config.resolve_workspace(workspace)?;
    let storage = storage::Root::open(&workspace)?;
    let may_write = |confirmed: bool| -> Result<()> {
        if config.access_mode() == AccessMode::ReadOnly {
            bail!("onboarding writes are disabled in read-only mode")
        }
        if !confirmed {
            bail!("creating a draft requires --confirm; preview stdout first")
        }
        Ok(())
    };
    match args.command {
        OnboardCommand::Inspect { json } => {
            let report = inspect(&workspace)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?)
            } else {
                print!("{}", render(&report))
            }
        }
        OnboardCommand::Preview {
            against,
            output,
            confirm,
        } => {
            let text = render(&inspect(&workspace)?);
            if text.len() > MAX_FILE {
                bail!("generated draft exceeds limit; select a narrower workspace")
            }
            if let Some(against) = against {
                let previous = String::from_utf8(storage.read(&against, MAX_FILE)?)?;
                if previous
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
                {
                    bail!("previous draft contains terminal control characters")
                }
                print!("{}", diffy::create_patch(&previous, &text));
            } else if let Some(output) = output {
                may_write(confirm)?;
                storage.publish(&output, text.as_bytes())?;
                println!(
                    "{}",
                    serde_json::json!({"status":"draft_created","sha256":digest(text.as_bytes())})
                );
            } else {
                print!("{text}")
            }
        }
        OnboardCommand::Accept {
            draft,
            sha256,
            output,
        } => {
            may_write(true)?;
            let bytes = storage.read(&draft, MAX_FILE)?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| anyhow::anyhow!("reviewed draft must be UTF-8"))?;
            if text.trim().is_empty()
                || text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
            {
                bail!("reviewed draft is empty or contains terminal control characters")
            }
            if sha256.len() != 64 || sha256 != digest(&bytes) {
                bail!(
                    "reviewed draft digest mismatch; inspect current content and confirm its SHA-256"
                )
            }
            storage.publish(&output, &bytes)?;
            println!(
                "{}",
                serde_json::json!({"status":"accepted","sha256":sha256})
            );
        }
    }
    Ok(())
}
#[cfg(test)]
mod guidance_tests;
#[cfg(test)]
mod tests;
