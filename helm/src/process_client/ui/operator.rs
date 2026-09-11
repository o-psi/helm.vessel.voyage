//! Bounded, live-schema operator forms. This module never sends an effect.
//! The caller must check the captured observation before using the ordinary durable
//! command path; closing a form is not cancelling an admitted command.
use super::state::Target;
use anyhow::{Context, Result, bail, ensure};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde_json::{Map, Value, json};
use uuid::Uuid;

const MAX_FIELDS: usize = 64;
const MAX_TEXT: usize = 16 * 1024;
const MAX_CHOICES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Observation {
    pub target: Target,
    pub incarnation: Uuid,
    pub revision: u64,
    pub run_id: Option<Uuid>,
}

pub(super) struct Request {
    pub observation: Observation,
    pub name: String,
    pub arguments: Value,
}
impl Request {
    /// Internal transport adapter only; JSON is never an operator input.
    pub fn command_text(&self) -> Result<String> {
        ensure!(
            !self.name.chars().any(char::is_whitespace),
            "Invalid registry tool name"
        );
        let text = format!("/tool {} {}", self.name, self.arguments);
        ensure!(
            text.len() <= 64 * 1024,
            "Tool arguments exceed command limit"
        );
        Ok(text)
    }
}

pub(super) enum Outcome {
    Stay,
    Close,
    Submit(Request),
}

#[derive(Clone)]
struct Choice {
    label: String,
    value: Value,
}
struct Field {
    name: String,
    schema: Value,
    required: bool,
    text: String,
    choices: Vec<Choice>,
    selected: Vec<usize>,
    cursor: usize,
    included: bool,
    unsupported: bool,
}
#[derive(Clone)]
struct Action {
    name: String,
    label: String,
    schema: Value,
}

pub(super) struct Panel {
    pub observation: Observation,
    actions: Vec<Action>,
    selected: usize,
    fields: Option<Vec<Field>>,
    focus: usize,
    reviewing: bool,
    error: String,
    tasks: Vec<Choice>,
    agents: Vec<Choice>,
    incomplete: bool,
}

fn value(envelope: &Value) -> &Value {
    envelope.get("value").unwrap_or(envelope)
}
fn entries(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(a) => a.iter().collect(),
        Value::Object(o) => o.values().collect(),
        _ => vec![],
    }
}
fn named(envelope: &Value, tasks: bool) -> Vec<Choice> {
    let v = value(envelope);
    let v = if tasks {
        v.get("items").unwrap_or(v)
    } else {
        v
    };
    entries(v)
        .into_iter()
        .take(MAX_CHOICES)
        .enumerate()
        .filter_map(|(index, item)| {
            let id = item.get("id")?.as_str()?;
            Uuid::parse_str(id).ok()?;
            let name = item[if tasks { "title" } else { "name" }]
                .as_str()
                .unwrap_or("Unnamed");
            let status = item["status"].as_str().unwrap_or("unknown");
            Some(Choice {
                label: format!("{}. {} ({})", index + 1, name, status),
                value: json!(id),
            })
        })
        .collect()
}
fn kind(schema: &Value) -> &str {
    schema["type"]
        .as_str()
        .or_else(|| {
            schema["type"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .find(|s| *s != "null")
        })
        .unwrap_or("")
}
fn merge(base: &Value, branch: &Value) -> Value {
    let mut result = base.as_object().cloned().unwrap_or_default();
    if let Some(fields) = branch.as_object() {
        result.extend(fields.clone());
    }
    Value::Object(result)
}

impl Panel {
    /// `filter` is None for manual tools, Some("todo") or Some("subagent")
    /// for management. Inventories and named choices must be from this observation.
    pub fn new(
        observation: Observation,
        tools: &Value,
        tasks: &Value,
        agents: &Value,
        filter: Option<&str>,
    ) -> Result<Self> {
        let inventory = value(tools);
        let inventory = inventory.get("inventory").unwrap_or(inventory);
        let incomplete = entries(inventory).len() > MAX_CHOICES
            || entries(value(tasks).get("items").unwrap_or(value(tasks))).len() > MAX_CHOICES
            || entries(value(agents)).len() > MAX_CHOICES
            || entries(inventory).iter().any(|t| {
                t["input_schema"]["oneOf"]
                    .as_array()
                    .is_some_and(|a| a.len() > MAX_CHOICES)
            });
        let mut actions = Vec::new();
        for tool in entries(inventory).into_iter().take(MAX_CHOICES) {
            let Some(name) = tool["name"].as_str() else {
                continue;
            };
            if filter.is_some_and(|f| f != name) {
                continue;
            }
            let schema = &tool["input_schema"];
            if let Some(branches) = schema["oneOf"].as_array() {
                for branch in branches.iter().take(MAX_CHOICES) {
                    let action = &branch["properties"]["action"];
                    let Some(action) = action["const"]
                        .as_str()
                        .or_else(|| action["enum"][0].as_str())
                    else {
                        continue;
                    };
                    let mut combined = branch.clone();
                    if let Some(props) = combined["properties"].as_object_mut() {
                        for (key, field) in props.iter_mut() {
                            *field = merge(&schema["properties"][key], field);
                        }
                    }
                    combined["properties"]["action"] = json!({"const":action});
                    actions.push(Action {
                        name: name.into(),
                        label: format!("{name} · {action}"),
                        schema: combined,
                    });
                }
            } else {
                actions.push(Action {
                    name: name.into(),
                    label: name.into(),
                    schema: schema.clone(),
                });
            }
        }
        ensure!(
            actions.len() <= 1024,
            "Tool registry exceeds form action limit"
        );
        actions.sort_by(|a, b| a.label.cmp(&b.label));
        Ok(Self {
            observation,
            actions,
            selected: 0,
            fields: None,
            focus: 0,
            reviewing: false,
            error: String::new(),
            tasks: named(tasks, true),
            agents: named(agents, false),
            incomplete,
        })
    }

    fn open(&mut self) -> Result<()> {
        let action = self
            .actions
            .get(self.selected)
            .context("No live tools available; start a run to initialize the registry")?;
        ensure!(
            action.schema["type"] == "object",
            "This tool does not expose a supported object form"
        );
        let props = action.schema["properties"]
            .as_object()
            .context("Tool has no supported fields")?;
        ensure!(props.len() <= MAX_FIELDS, "Tool exceeds form field limit");
        let required = action.schema["required"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut fields = vec![];
        for (name, schema) in props {
            if schema.get("const").is_some() {
                continue;
            }
            let scalar = if kind(schema) == "array" {
                &schema["items"]
            } else {
                schema
            };
            let is_uuid = scalar["format"] == "uuid";
            let choices = if is_uuid {
                match action.name.as_str() {
                    "todo" => self.tasks.clone(),
                    "subagent" => self.agents.clone(),
                    _ => vec![],
                }
            } else if let Some(options) = scalar["enum"].as_array() {
                options
                    .iter()
                    .filter(|v| !v.is_null())
                    .map(|v| Choice {
                        label: v
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string()),
                        value: v.clone(),
                    })
                    .collect()
            } else if kind(scalar) == "boolean" {
                vec![
                    Choice {
                        label: "Yes".into(),
                        value: json!(true),
                    },
                    Choice {
                        label: "No".into(),
                        value: json!(false),
                    },
                ]
            } else {
                vec![]
            };
            let unsupported = (is_uuid && choices.is_empty())
                || (!is_uuid
                    && choices.is_empty()
                    && !matches!(kind(scalar), "string" | "integer" | "number"))
                || name == "workflow_secrets"
                || schema["secret"] == true
                || schema["writeOnly"] == true;
            fields.push(Field {
                name: name.clone(),
                schema: schema.clone(),
                required: required.contains(&json!(name)),
                text: String::new(),
                choices,
                selected: vec![],
                cursor: 0,
                included: false,
                unsupported,
            });
        }
        self.fields = Some(fields);
        self.focus = 0;
        Ok(())
    }

    fn arguments(&self) -> Result<Value> {
        let action = &self.actions[self.selected];
        let mut arguments = Map::new();
        if let Some(properties) = action.schema["properties"].as_object() {
            for (key, schema) in properties {
                if let Some(v) = schema.get("const") {
                    arguments.insert(key.clone(), v.clone());
                }
            }
        }
        for field in self.fields.as_ref().context("Choose a tool first")? {
            if !field.included {
                ensure!(!field.required, "{} is required", field.name);
                continue;
            }
            ensure!(
                !field.unsupported,
                "{} cannot be entered safely in this form",
                field.name
            );
            let v = if !field.choices.is_empty() {
                let values: Vec<_> = field
                    .selected
                    .iter()
                    .map(|i| field.choices[*i].value.clone())
                    .collect();
                if kind(&field.schema) == "array" {
                    Value::Array(values)
                } else {
                    values
                        .first()
                        .cloned()
                        .context(format!("Choose {}", field.name))?
                }
            } else if kind(&field.schema) == "array" {
                let items = if field.text.is_empty() {
                    vec![]
                } else {
                    field
                        .text
                        .lines()
                        .map(|line| scalar(line, &field.schema["items"]))
                        .collect::<Result<Vec<_>>>()?
                };
                Value::Array(items)
            } else {
                scalar(&field.text, &field.schema)?
            };
            validate(&v, &field.schema).with_context(|| format!("Invalid {}", field.name))?;
            arguments.insert(field.name.clone(), v);
        }
        Ok(Value::Object(arguments))
    }

    pub fn input(&mut self, event: &Event) -> Outcome {
        self.error.clear();
        match self.handle(event) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.error = format!("{error:#}");
                Outcome::Stay
            }
        }
    }
    fn handle(&mut self, event: &Event) -> Result<Outcome> {
        if let Event::Paste(text) = event {
            if !self.reviewing
                && let Some(field) = self.fields.as_mut().and_then(|f| f.get_mut(self.focus))
            {
                field.append(text)?;
            }
            return Ok(Outcome::Stay);
        }
        let Event::Key(key) = event else {
            return Ok(Outcome::Stay);
        };
        if key.kind == KeyEventKind::Release {
            return Ok(Outcome::Stay);
        }
        if key.code == KeyCode::Esc {
            if self.reviewing {
                self.reviewing = false;
            } else if self.fields.is_some() {
                self.fields = None;
            } else {
                return Ok(Outcome::Close);
            }
            return Ok(Outcome::Stay);
        }
        if self.reviewing {
            if key.code == KeyCode::Enter {
                return Ok(Outcome::Submit(Request {
                    observation: self.observation,
                    name: self.actions[self.selected].name.clone(),
                    arguments: self.arguments()?,
                }));
            }
            return Ok(Outcome::Stay);
        }
        let Some(fields) = self.fields.as_mut() else {
            match key.code {
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.actions.len().saturating_sub(1))
                }
                KeyCode::Enter => self.open()?,
                _ => {}
            }
            return Ok(Outcome::Stay);
        };
        if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.arguments()?;
            self.reviewing = true;
            return Ok(Outcome::Stay);
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1).min(fields.len()),
            KeyCode::BackTab | KeyCode::Up => self.focus = self.focus.saturating_sub(1),
            KeyCode::Enter if self.focus == fields.len() => {
                self.arguments()?;
                self.reviewing = true;
            }
            _ => {
                if let Some(field) = fields.get_mut(self.focus) {
                    match key.code {
                        KeyCode::Left => field.cursor = field.cursor.saturating_sub(1),
                        KeyCode::Right => {
                            field.cursor =
                                (field.cursor + 1).min(field.choices.len().saturating_sub(1))
                        }
                        KeyCode::Enter | KeyCode::Char(' ') if !field.choices.is_empty() => {
                            field.included = true;
                            if kind(&field.schema) != "array" {
                                field.selected.clear();
                            }
                            if let Some(index) =
                                field.selected.iter().position(|i| *i == field.cursor)
                            {
                                field.selected.remove(index);
                            } else {
                                field.selected.push(field.cursor);
                            }
                        }
                        KeyCode::Delete => {
                            field.included = false;
                            field.text.clear();
                            field.selected.clear();
                        }
                        KeyCode::Backspace => {
                            field.text.pop();
                        }
                        KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
                            field.append("\n")?
                        }
                        KeyCode::Enter => {
                            ensure!(!field.unsupported, "Field unavailable");
                            field.included = true;
                        }
                        KeyCode::Char(c)
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                        {
                            field.append(&c.to_string())?
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(Outcome::Stay)
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let safe = crate::process_client::safe;
        let mut lines =
            vec!["Operator action · runtime permissions and approvals still apply".to_owned()];
        if self.incomplete {
            lines.push("INCOMPLETE: inventory or named choices exceed 256 entries; omitted entries are not selectable.".into());
        }
        if self.reviewing {
            lines.push(format!(
                "Review {} · Enter executes once; Esc edits",
                self.actions[self.selected].label
            ));
            lines.push(
                "Public arguments are recorded in voyage history. Never enter passwords or tokens."
                    .into(),
            );
            if let Some(fields) = &self.fields {
                for field in fields.iter().filter(|f| f.included) {
                    lines.push(format!("{}: {}", field.name, field.display()));
                }
            }
        } else if let Some(fields) = &self.fields {
            lines.push(format!(
                "{} · Tab fields · ←/→ choices · Space select",
                self.actions[self.selected].label
            ));
            lines.push("Alt+Enter newline · Delete omit · Ctrl+Enter review · Esc back".into());
            let visible = usize::from(area.height.saturating_sub(8)).max(1);
            let start = self.focus.saturating_sub(visible / 2);
            for (index, field) in fields.iter().enumerate().skip(start).take(visible) {
                lines.push(format!(
                    "{} {}{}: {}",
                    if self.focus == index { "›" } else { " " },
                    field.name,
                    if field.required { " *" } else { "" },
                    field.display()
                ));
            }
            lines.push(format!(
                "{} Review action",
                if self.focus == fields.len() {
                    "›"
                } else {
                    " "
                }
            ));
            if let Some(field) = fields.get(self.focus) {
                if let Some(description) = field.schema["description"].as_str() {
                    lines.push(description.into());
                }
                if let Some(choice) = field.choices.get(field.cursor) {
                    lines.push(format!(
                        "Choice {}/{}: {} · Space selects",
                        field.cursor + 1,
                        field.choices.len(),
                        choice.label
                    ));
                }
                if kind(&field.schema) == "array" && field.choices.is_empty() {
                    lines.push(
                        "One item per line; Enter includes an empty list; Delete omits.".into(),
                    );
                }
            }
        } else {
            lines.push("Choose a live tool action · ↑/↓ · Enter · Esc closes".into());
            if self.actions.is_empty() {
                lines.push("No matching tools. Initialize a run, then reopen to refresh the live registry.".into());
            }
            let visible = usize::from(area.height.saturating_sub(6)).max(1);
            for (index, action) in self
                .actions
                .iter()
                .enumerate()
                .skip(self.selected.saturating_sub(visible / 2))
                .take(visible)
            {
                lines.push(format!(
                    "{} {}",
                    if index == self.selected { "›" } else { " " },
                    action.label
                ));
            }
        }
        if !self.error.is_empty() {
            lines.push(format!("Error: {}", self.error));
        }
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(safe(&lines.join("\n")))
                .block(
                    Block::default()
                        .title(" Operator actions ")
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            area,
        );
    }
}
impl Field {
    fn append(&mut self, text: &str) -> Result<()> {
        ensure!(
            !self.unsupported,
            "Field unavailable: unsupported structure, private input, or no named choices"
        );
        ensure!(
            self.choices.is_empty(),
            "Use left/right and Space to choose a value"
        );
        ensure!(
            self.text.len().saturating_add(text.len()) <= MAX_TEXT,
            "Field input limit is 16 KiB"
        );
        ensure!(
            !text
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t'),
            "Control characters are not accepted"
        );
        self.text.push_str(text);
        self.included = true;
        Ok(())
    }
    fn display(&self) -> String {
        if self.unsupported {
            return "Unavailable (unsupported/private input or no named choices)".into();
        }
        if !self.included {
            return "Not supplied".into();
        }
        if !self.choices.is_empty() {
            return self
                .selected
                .iter()
                .map(|i| self.choices[*i].label.clone())
                .collect::<Vec<_>>()
                .join(", ");
        }
        if self.text.is_empty() {
            "Empty (explicit)".into()
        } else {
            self.text.replace('\n', " ↵ ")
        }
    }
}
fn scalar(text: &str, schema: &Value) -> Result<Value> {
    match kind(schema) {
        "string" => Ok(json!(text)),
        "integer" => text
            .parse::<i64>()
            .map(|n| json!(n))
            .context("Enter an integer"),
        "number" => {
            let n = text.parse::<f64>().context("Enter a number")?;
            ensure!(n.is_finite(), "Number must be finite");
            Ok(json!(n))
        }
        _ => bail!("Unsupported field type"),
    }
}
fn validate(value: &Value, schema: &Value) -> Result<()> {
    if let Some(s) = value.as_str() {
        let len = s.chars().count() as u64;
        ensure!(
            schema["minLength"].as_u64().is_none_or(|n| len >= n),
            "Text is too short"
        );
        ensure!(
            schema["maxLength"].as_u64().is_none_or(|n| len <= n),
            "Text is too long"
        );
        if schema["pattern"] == "\\S" {
            ensure!(!s.trim().is_empty(), "Text must not be blank");
        }
    }
    if let Some(n) = value.as_f64() {
        ensure!(
            schema["minimum"].as_f64().is_none_or(|bound| n >= bound),
            "Number is below minimum"
        );
        ensure!(
            schema["maximum"].as_f64().is_none_or(|bound| n <= bound),
            "Number exceeds maximum"
        );
    }
    if let Some(items) = value.as_array() {
        ensure!(items.len() <= MAX_CHOICES, "Array exceeds form limit");
        ensure!(
            schema["minItems"]
                .as_u64()
                .is_none_or(|n| items.len() as u64 >= n),
            "Too few items"
        );
        ensure!(
            schema["maxItems"]
                .as_u64()
                .is_none_or(|n| items.len() as u64 <= n),
            "Too many items"
        );
        for (i, item) in items.iter().enumerate() {
            if schema["uniqueItems"] == true {
                ensure!(!items[..i].contains(item), "Duplicate items");
            }
            validate(item, &schema["items"])?;
        }
    }
    // Voyage validates the complete live schema and enforces authority before effects.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    fn observation() -> Observation {
        Observation {
            target: Target {
                route: super::super::state::Route {
                    id: Uuid::new_v4(),
                    generation: 1,
                },
                session: Uuid::new_v4(),
            },
            incarnation: Uuid::new_v4(),
            revision: 7,
            run_id: None,
        }
    }
    fn press(panel: &mut Panel, code: KeyCode, modifiers: KeyModifiers) -> Outcome {
        panel.input(&Event::Key(KeyEvent::new(code, modifiers)))
    }
    fn panel(schema: Value) -> Panel {
        Panel::new(
            observation(),
            &json!([{"name":"read_file","input_schema":schema}]),
            &Value::Null,
            &Value::Null,
            None,
        )
        .unwrap()
    }

    #[test]
    fn form_requires_review_and_preserves_observation_and_typed_text() {
        let mut panel = panel(
            json!({"type":"object","properties":{"path":{"type":"string","minLength":1}},"required":["path"]}),
        );
        let captured = panel.observation;
        assert!(matches!(
            press(&mut panel, KeyCode::Enter, KeyModifiers::NONE),
            Outcome::Stay
        ));
        press(&mut panel, KeyCode::Enter, KeyModifiers::CONTROL);
        assert!(!panel.reviewing);
        panel.input(&Event::Paste("folder/a file.txt".into()));
        assert!(matches!(
            press(&mut panel, KeyCode::Enter, KeyModifiers::CONTROL),
            Outcome::Stay
        ));
        assert!(panel.reviewing);
        let Outcome::Submit(request) = press(&mut panel, KeyCode::Enter, KeyModifiers::NONE) else {
            panic!("expected submission")
        };
        assert_eq!(request.observation, captured);
        assert_eq!(request.arguments, json!({"path":"folder/a file.txt"}));
        assert_eq!(
            request.command_text().unwrap(),
            "/tool read_file {\"path\":\"folder/a file.txt\"}"
        );
    }

    #[test]
    fn action_union_merges_common_constraints_and_uses_named_ids() {
        let id = Uuid::new_v4();
        let tools = json!({"value":{"inventory":[{"name":"todo","input_schema":{
            "type":"object","properties":{"id":{"type":"string","format":"uuid"},"status":{"type":"string","enum":["pending","completed"]}},
            "oneOf":[{"type":"object","properties":{"action":{"enum":["status"]},"id":{},"status":{}},"required":["action","id","status"]}]
        }}]}});
        let mut panel = Panel::new(
            observation(),
            &tools,
            &json!({"value":{"items":[{"id":id,"title":"Ship fix","status":"pending"}]}}),
            &Value::Null,
            Some("todo"),
        )
        .unwrap();
        panel.open().unwrap();
        let fields = panel.fields.as_mut().unwrap();
        let identity = fields.iter_mut().find(|f| f.name == "id").unwrap();
        assert!(identity.choices[0].label.contains("Ship fix"));
        assert!(!identity.choices[0].label.contains(&id.to_string()));
        assert!(identity.append("raw-id").is_err());
        identity.included = true;
        identity.selected.push(0);
        let status = fields.iter_mut().find(|f| f.name == "status").unwrap();
        status.included = true;
        status.selected.push(1);
        assert_eq!(
            panel.arguments().unwrap(),
            json!({"action":"status","id":id,"status":"completed"})
        );
    }

    #[test]
    fn omission_empty_arrays_and_bounded_paste_are_distinct() {
        let mut panel = panel(
            json!({"type":"object","properties":{"items":{"type":"array","items":{"type":"string"}}}}),
        );
        panel.open().unwrap();
        assert_eq!(panel.arguments().unwrap(), json!({}));
        press(&mut panel, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(panel.arguments().unwrap(), json!({"items":[]}));
        panel.input(&Event::Paste("a\nb".into()));
        assert_eq!(panel.arguments().unwrap(), json!({"items":["a","b"]}));
        panel.input(&Event::Paste("x".repeat(MAX_TEXT)));
        assert!(!panel.error.is_empty());
        assert_eq!(panel.arguments().unwrap(), json!({"items":["a","b"]}));
        press(&mut panel, KeyCode::Delete, KeyModifiers::NONE);
        assert_eq!(panel.arguments().unwrap(), json!({}));
    }

    #[test]
    fn unsupported_required_private_and_reference_fields_fail_closed() {
        for schema in [
            json!({"type":"object"}),
            json!({"type":"string","format":"uuid"}),
            json!({"type":"string","secret":true}),
        ] {
            let mut panel =
                panel(json!({"type":"object","properties":{"value":schema},"required":["value"]}));
            panel.open().unwrap();
            panel.input(&Event::Paste("secret".into()));
            assert!(!panel.error.is_empty());
            assert!(panel.arguments().is_err());
        }
    }

    #[test]
    fn empty_runtime_inventory_is_not_replaced_with_invented_tools() {
        let mut panel = Panel::new(
            observation(),
            &json!({"value":{"inventory":[],"source":"runtime_not_constructed"}}),
            &Value::Null,
            &Value::Null,
            None,
        )
        .unwrap();
        assert!(panel.actions.is_empty());
        assert!(panel.open().is_err());
    }

    #[test]
    fn local_bounds_and_duplicate_validation() {
        assert!(validate(&json!(0), &json!({"minimum":1})).is_err());
        assert!(validate(&json!(["a", "a"]), &json!({"uniqueItems":true})).is_err());
        assert!(validate(&json!("  "), &json!({"pattern":"\\S"})).is_err());
        assert!(scalar("NaN", &json!({"type":"number"})).is_err());
    }
}
