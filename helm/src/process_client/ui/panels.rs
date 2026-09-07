//! User-facing summaries. Wire schemas and ownership bookkeeping belong in diagnostics.
use super::{
    presentation::{fields, label},
    safe,
};
use serde_json::Value;

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(safe)
        .unwrap_or_default()
}
fn entries(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(items) => items.values().collect(),
        _ => Vec::new(),
    }
}
fn item(output: &mut String, title: &str, detail: &str) {
    output.push_str(&format!("## {}\n\n{}\n\n", safe(title), safe(detail)));
}

pub(super) fn render(section: &str, envelope: &Value) -> String {
    let value = envelope.get("value").unwrap_or(envelope);
    let mut output = String::new();
    match section {
        "tools" => {
            output.push_str("# Available tools\n\nThese are the tools this voyage can use.\n\n");
            let tools = value.get("inventory").unwrap_or(value);
            for tool in entries(tools).into_iter().take(256) {
                let name = text(tool, "name");
                let description = match name.as_str() {
                    "shell" => "Run a one-off command.",
                    "process" => "Run interactive programs. Open their consoles with F3.",
                    "questions" => "Ask you a question with choices or a written answer.",
                    "todo" => "Keep track of tasks, progress and anything blocking your work.",
                    _ => tool["description"]
                        .as_str()
                        .unwrap_or("Available to this voyage."),
                };
                item(&mut output, &label(&name), description);
            }
            if entries(tools).is_empty() {
                output.push_str("No tools available yet. The list refreshes when a run starts.\n");
            }
        }
        "todos" => {
            output.push_str("# Tasks\n\n");
            let tasks = entries(&value["items"]);
            if tasks.is_empty() {
                output.push_str("No tasks yet. Tasks created during work will appear here.\n");
            }
            for task in tasks.into_iter().take(256) {
                let detail = format!(
                    "{}\n\n{}",
                    label(&text(task, "status")),
                    text(task, "description")
                );
                item(&mut output, &text(task, "title"), &detail);
                if let Some(blockers) = task["blockers"].as_array() {
                    for blocker in blockers.iter().filter_map(Value::as_str) {
                        output.push_str(&format!("Waiting on: {}\n\n", safe(blocker)));
                    }
                }
            }
        }
        "subagents" => {
            output.push_str("# Agents\n\n");
            let agents = entries(value);
            if agents.is_empty() {
                output.push_str("No agents working on this voyage.\n");
            }
            for agent in agents.into_iter().take(256) {
                item(
                    &mut output,
                    &text(agent, "name"),
                    &format!(
                        "{}\n\n{}",
                        label(&text(agent, "status")),
                        text(agent, "task")
                    ),
                );
                for key in ["result", "error"] {
                    let detail = text(agent, key);
                    if !detail.is_empty() {
                        output.push_str(&format!("{detail}\n\n"));
                    }
                }
            }
        }
        "models" => {
            output.push_str("# Models\n\nUse /model followed by a model name to change it.\n\n");
            let models = entries(value);
            if models.is_empty() {
                output.push_str("No models were returned by this provider.\n");
            }
            for model in models.into_iter().take(256) {
                let name = text(model, "display_name");
                let id = text(model, "id");
                item(
                    &mut output,
                    if name.is_empty() { &id } else { &name },
                    &text(model, "description"),
                );
                if name != id {
                    output.push_str(&format!("Model name: {id}\n\n"));
                }
            }
        }
        "workflows" => {
            output.push_str("# Saved workflows\n\n");
            let workflows = entries(value);
            if workflows.is_empty() {
                output.push_str("No saved workflows in this workspace.\n");
            }
            for workflow in workflows.into_iter().take(256) {
                let document = &workflow["document"];
                item(
                    &mut output,
                    &text(document, "id"),
                    &text(document, "description"),
                );
            }
        }
        "policy" => {
            output.push_str("# Permissions\n\n");
            let rules = value.get("rules").unwrap_or(value);
            let access = text(rules, "access");
            let description = match access.as_str() {
                "read-only" | "read_only" => {
                    "Read only: changes and commands that modify your work are restricted."
                }
                "approval" => "Ask first: actions requiring permission are shown for your review.",
                "unrestricted" => {
                    "Unrestricted: actions may run without asking, within this machine's configured limits."
                }
                _ => "Permissions are set on the machine running this voyage.",
            };
            output.push_str(description);
            output.push_str("\n\n");
            for (key, title) in [
                ("read_roots", "Readable folders"),
                ("write_roots", "Writable folders"),
                ("deny_commands", "Blocked commands"),
            ] {
                if let Some(items) = rules[key].as_array()
                    && !items.is_empty()
                {
                    output.push_str(&format!("## {title}\n\n"));
                    for entry in items.iter().filter_map(Value::as_str) {
                        output.push_str(&format!("- {}\n", safe(entry)));
                    }
                    output.push('\n');
                }
            }
        }
        "host_resources" => {
            output.push_str(
                "# Machine capacity\n\nLimits shared by work running on this machine.\n\n",
            );
            for (key, title) in [
                ("executors", "Concurrent runs"),
                ("terminals", "Interactive terminals"),
            ] {
                if let Some(limit) = value["limits"][key].as_u64() {
                    output.push_str(&format!("{title}: up to {limit}\n\n"));
                }
            }
            output.push_str("Capacity is released after cleanup is confirmed. These limits do not indicate how much work has completed.\n");
        }
        _ => {
            output.push_str(&fields(value));
        }
    }
    output
}
