//! Bounded convention evidence, not a natural-language instruction interpreter.
use super::{Candidate, Project, Report};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_LINES: usize = 512;
const MAX_LINE: usize = 4096;
const MAX_COMMANDS: usize = 64;
const RUNNERS: &[&str] = &["npm", "pnpm", "yarn", "bun"];

fn family(word: &str) -> Option<&'static str> {
    match word {
        "npm" | "pnpm" | "yarn" | "bun" => Some("JavaScript/TypeScript"),
        "cargo" => Some("Rust"),
        "python" | "python3" | "pytest" | "ruff" | "uv" | "poetry" => Some("Python"),
        "go" => Some("Go"),
        _ => None,
    }
}

pub(super) struct Guidance {
    path: String,
    limited: bool,
    negative: bool,
    ambiguous_context: bool,
    mentions: BTreeSet<&'static str>,
    managers: BTreeSet<String>,
    commands: Vec<Vec<String>>,
}

impl Guidance {
    pub fn unreadable(path: String) -> Self {
        Self {
            path,
            limited: true,
            negative: false,
            ambiguous_context: false,
            mentions: BTreeSet::new(),
            managers: BTreeSet::new(),
            commands: Vec::new(),
        }
    }

    pub fn inspect(path: String, bytes: &[u8]) -> Self {
        let mut result = Self::unreadable(path);
        let Ok(text) = std::str::from_utf8(bytes) else {
            return result;
        };
        result.limited = false;
        let mut in_fence = false;
        for (index, line) in text.lines().enumerate() {
            if index >= MAX_LINES || line.len() > MAX_LINE {
                result.limited = true;
                break;
            }
            if line.chars().any(|c| c.is_control() && c != '\t') {
                result.limited = true;
                break;
            }
            let trimmed = line.trim();
            if let Some(language) = trimmed.strip_prefix("```") {
                let allowed = if in_fence {
                    language.is_empty()
                } else {
                    matches!(language, "" | "sh" | "bash" | "shell" | "console")
                };
                result.ambiguous_context |= !allowed;
                in_fence = !in_fence;
                continue;
            }
            if line.contains("<!--")
                || line.contains("-->")
                || trimmed.starts_with('>')
                || trimmed.starts_with("~~~")
                || matches!(trimmed, "\"" | "'" | "“" | "”" | "‘" | "’")
            {
                result.ambiguous_context = true;
            }
            let lower = line.to_ascii_lowercase().replace('’', "'");
            for word in lower.split(|c: char| !c.is_ascii_alphanumeric() && c != '\'') {
                if let Some(family) = family(word) {
                    result.mentions.insert(family);
                }
                if RUNNERS.contains(&word) {
                    result.managers.insert(word.to_owned());
                }
                if matches!(
                    word,
                    "not"
                        | "never"
                        | "avoid"
                        | "don't"
                        | "unsupported"
                        | "forbidden"
                        | "no"
                        | "instead"
                        | "example"
                        | "examples"
                        | "incorrect"
                        | "obsolete"
                        | "deprecated"
                        | "prohibited"
                        | "cannot"
                        | "can't"
                        | "mustn't"
                        | "caution"
                        | "warning"
                        | "disabled"
                ) {
                    result.negative = true;
                }
            }
            // Only a standalone command or a complete, explicit Run directive.
            // Quotes, surrounding explanation, shell composition and unsupported
            // arguments cannot become a positive recommendation through substring extraction.
            let line = line.trim();
            let line = line
                .strip_prefix("Run ")
                .or_else(|| line.strip_prefix("run "))
                .unwrap_or(line);
            // Strip prose punctuation only outside a complete code span;
            // preserve argument bytes such as the standalone '.' path.
            let line = line
                .strip_suffix('.')
                .filter(|_| line.ends_with("`."))
                .unwrap_or(line);
            let line = line
                .strip_prefix('`')
                .and_then(|s| s.strip_suffix('`'))
                .unwrap_or(line);
            if line.len() > 256 {
                continue;
            }
            if let Ok(args) = shell_words::split(line)
                && args.first().is_some_and(|arg| family(arg).is_some())
            {
                if result.commands.len() == MAX_COMMANDS {
                    result.limited = true;
                    break;
                }
                result.commands.push(args);
            }
        }
        result.ambiguous_context |= in_fence;
        result
    }
}

fn applies(path: &str, project: &str) -> bool {
    let directory = Path::new(path).parent().unwrap_or(Path::new(""));
    let project = if project == "." {
        Path::new("")
    } else {
        Path::new(project)
    };
    project.starts_with(directory)
}

fn supported(project: &Project, args: &[String], scripts: &BTreeSet<&str>) -> bool {
    if project.ecosystem == "JavaScript/TypeScript" {
        return args.len() == 3
            && RUNNERS.contains(&args[0].as_str())
            && args[1] == "run"
            && !args[2].is_empty()
            && !args[2].starts_with('-')
            && args[2].len() <= 128
            && args[2]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_:./".contains(c))
            && scripts.contains(args[2].as_str());
    }
    project
        .commands
        .iter()
        .any(|candidate| candidate.argv == args)
}

pub(super) fn reconcile(
    report: &mut Report,
    guidance: &[Guidance],
    manifests: &BTreeMap<String, Vec<u8>>,
) {
    let active: Vec<_> = guidance.iter().filter(|item| {
        if Path::new(&item.path).file_name().is_some_and(|name| name == "agents.md")
            && guidance.iter().any(|other| Path::new(&other.path) == Path::new(&item.path).with_file_name("AGENTS.md"))
        {
            report.warnings.push(format!("{} is shadowed by AGENTS.md; its conventions were not used", item.path));
            false
        } else {
            report.warnings.push(format!("Existing guidance {} requires manual review; bounded command evidence does not interpret its prose", item.path));
            true
        }
    }).collect();
    for project in &mut report.projects {
        let relevant: Vec<_> = active
            .iter()
            .copied()
            .filter(|item| applies(&item.path, &project.directory))
            .collect();
        let source = manifests
            .keys()
            .find(|source| {
                Path::new(source).parent().unwrap_or(Path::new(""))
                    == if project.directory == "." {
                        Path::new("")
                    } else {
                        Path::new(&project.directory)
                    }
                    && match project.ecosystem.as_str() {
                        "JavaScript/TypeScript" => source.ends_with("package.json"),
                        "Rust" => source.ends_with("Cargo.toml"),
                        "Python" => source.ends_with("pyproject.toml"),
                        "Go" => source.ends_with("go.mod"),
                        _ => false,
                    }
            })
            .expect("project has its parsed manifest");
        let manifest = manifests.get(source).map(Vec::as_slice);
        let mentioned: Vec<_> = relevant
            .iter()
            .copied()
            .filter(|item| item.mentions.contains(project.ecosystem.as_str()))
            .collect();
        let managers: BTreeSet<_> = mentioned
            .iter()
            .flat_map(|item| item.managers.iter())
            .collect();
        // Parse each manifest once, not once per untrusted command snippet.
        let metadata =
            manifest.and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok());
        let declared_manager = metadata
            .as_ref()
            .and_then(|value| value.get("packageManager")?.as_str())
            .and_then(|value| value.split('@').next());
        let scripts: BTreeSet<_> = metadata
            .as_ref()
            .and_then(|value| value.get("scripts")?.as_object())
            .into_iter()
            .flat_map(|scripts| scripts.iter())
            .filter_map(|(name, value)| value.is_string().then_some(name.as_str()))
            .collect();
        let commands: Vec<_> = mentioned
            .iter()
            .flat_map(|item| {
                item.commands
                    .iter()
                    .filter(|args| {
                        args.first().and_then(|arg| family(arg)) == Some(project.ecosystem.as_str())
                    })
                    .map(|args| (*item, args))
            })
            .collect();
        let mut reasons = Vec::new();
        if managers.len() > 1 {
            reasons.push("multiple package managers mentioned");
        }
        if declared_manager
            .is_some_and(|declared| managers.iter().any(|manager| manager.as_str() != declared))
        {
            reasons.push("guidance and packageManager disagree");
        }
        if relevant.iter().any(|item| item.limited) {
            reasons.push("guidance could not be inspected safely within bounds");
        }
        if mentioned.iter().any(|item| item.negative) {
            reasons.push("caution, negation or illustrative examples need interpretation");
        }
        if mentioned.iter().any(|item| item.ambiguous_context) {
            reasons.push("quoted, commented or unsupported Markdown context needs interpretation");
        }
        if !mentioned.is_empty() && commands.is_empty() {
            reasons.push("no supported standalone command for the mentioned convention");
        }
        if commands
            .iter()
            .any(|(_, args)| !supported(project, args, &scripts))
        {
            reasons.push("unsupported command shape or undeclared script");
        }
        if !reasons.is_empty() {
            project.commands.clear();
            let sources = relevant
                .iter()
                .map(|item| item.path.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            report.warnings.push(format!("{} in {}: guidance conflict or uncertainty ({sources}): {}; candidate commands omitted for manual review", project.ecosystem, project.directory, reasons.join("; ")));
        } else if !commands.is_empty() {
            let mut documented = BTreeMap::<Vec<String>, BTreeSet<&str>>::new();
            for (item, args) in commands {
                documented
                    .entry(args.clone())
                    .or_default()
                    .insert(&item.path);
            }
            project.commands = documented.into_iter().map(|(argv, evidence)| Candidate {
                argv,
                evidence: std::iter::once(source.as_str()).chain(evidence).collect::<Vec<_>>().join(", "),
                confidence: "documented command shape with manifest evidence; review its meaning and implementation".into(),
                verified: false,
            }).collect();
        }
    }
}
