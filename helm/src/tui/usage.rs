//! Read-only historical accounting; navigation never starts inference.
use super::{App, TitleJob, UiEvent, publication, text::display_safe};
use crate::{
    Agent,
    inference::history::{GroupBy, History, Query, Tokens},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub(super) fn default_query() -> Query {
    let until = chrono::Utc::now();
    Query {
        from: until - chrono::Duration::days(7),
        until,
        group_by: GroupBy::Model,
        detail: None,
        offset: 0,
        limit: 50,
        snapshot: None,
    }
}
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    pub authority: Option<Arc<Agent>>,
    session: Option<Uuid>,
    request: Option<Uuid>,
    job: Option<TitleJob>,
    query: Option<Query>,
    project_scope: bool,
    data: Option<History>,
    error: Option<String>,
    selected: usize,
    scroll: usize,
}
impl Panel {
    pub fn matches(&self, session: Uuid, request: Uuid) -> bool {
        self.open && self.session == Some(session) && self.request == Some(request)
    }
    pub fn close(&mut self) {
        self.job = None;
        self.request = None;
        self.open = false;
        self.authority = None;
        self.data = None;
        self.error = None;
    }
    pub fn finished(&mut self, result: Result<History, String>) {
        self.job = None;
        self.request = None;
        self.selected = 0;
        self.scroll = 0;
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
            }
            Err(error) => {
                self.data = None;
                self.error = Some(error);
            }
        }
    }
    fn reload(&self, mut query: Query) -> Option<(Query, bool)> {
        query.offset = 0;
        query.snapshot = None;
        Some((query, self.project_scope))
    }
    pub fn key(&mut self, key: KeyEvent, area: Rect) -> Option<(Query, bool)> {
        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            self.close();
            return None;
        }
        if self.request.is_some() {
            return None;
        }
        let mut query = self.query.clone()?;
        if let KeyCode::Char(number @ '1'..='5') = key.code {
            query.group_by = match number {
                '1' => GroupBy::Session,
                '2' => GroupBy::Model,
                '3' => GroupBy::Agent,
                '4' => GroupBy::Purpose,
                _ => GroupBy::Day,
            };
            query.detail = None;
            return self.reload(query);
        }
        match key.code {
            KeyCode::Char('r') => return self.reload(query),
            KeyCode::Char('s') => {
                query.offset = 0;
                query.snapshot = None;
                query.detail = None;
                return Some((query, !self.project_scope));
            }
            KeyCode::Backspace if query.detail.is_some() => {
                query.detail = None;
                return self.reload(query);
            }
            _ => {}
        }
        let data = self.data.as_ref()?;
        match key.code {
            KeyCode::Char('n') => {
                let cursor = data.next.as_ref()?;
                query.offset = cursor.offset;
                query.snapshot = Some(cursor.snapshot.clone());
                return Some((query, self.project_scope));
            }
            KeyCode::Char('p') if query.offset > 0 => {
                query.offset = query.offset.saturating_sub(query.limit);
                query.snapshot = Some(data.snapshot.clone());
                return Some((query, self.project_scope));
            }
            KeyCode::Enter if query.detail.is_none() => {
                query.detail = Some(data.groups.get(self.selected)?.key.clone());
                return self.reload(query);
            }
            KeyCode::Up if query.detail.is_none() => {
                self.selected = self.selected.saturating_sub(1)
            }
            KeyCode::Down if query.detail.is_none() => {
                self.selected = (self.selected + 1).min(data.groups.len().saturating_sub(1))
            }
            _ => {
                let maximum =
                    publication::lines(&self.text(), area.width.saturating_sub(2) as usize)
                        .len()
                        .saturating_sub(area.height.saturating_sub(2) as usize);
                publication::navigate(
                    key,
                    &mut self.scroll,
                    maximum,
                    area.height.saturating_sub(2) as usize,
                );
            }
        }
        if query.detail.is_none() && matches!(key.code, KeyCode::Up | KeyCode::Down) {
            let lines = publication::lines(&self.text(), area.width.saturating_sub(2) as usize);
            if let Some(line) = lines.iter().position(|line| line.starts_with('▶')) {
                let height = area.height.saturating_sub(2).max(1) as usize;
                if line < self.scroll {
                    self.scroll = line;
                } else if line >= self.scroll + height {
                    self.scroll = line + 1 - height;
                }
            }
        }
        None
    }
    fn text(&self) -> String {
        let Some(query) = &self.query else {
            return String::new();
        };
        let mut text = format!(
            "{} history · [{}, {}) UTC\n1 session · 2 model · 3 agent · 4 purpose · 5 daily trend\ns project/voyage · r refresh · n/p page · Enter details · Backspace groups\n",
            if self.project_scope {
                "Project"
            } else {
                "Voyage"
            },
            query.from.to_rfc3339(),
            query.until.to_rfc3339()
        );
        if self.request.is_some() {
            text.push_str("Loading historical accounting… Esc cancels.\n");
            return text;
        }
        if let Some(error) = &self.error {
            text.push_str(&format!(
                "{}\nRefresh to observe a new snapshot; earlier pages are not combined.\n",
                display_safe(error)
            ));
            return text;
        }
        let Some(data) = &self.data else { return text };
        text.push_str(&format!("Retained {} · completed {} · failed {} · unknown {}\nInput {}\nOutput {}\nLifetime permits {} · omitted detail {}\n",data.totals.retained_attempts,data.totals.completed,data.totals.failed,data.totals.unknown,tokens(&data.totals.input),tokens(&data.totals.output),data.scope_lifetime_permits,data.scope_lifetime_omitted_details));
        if data.range_permits.is_none() {
            text.push_str(
                "Range total unavailable: omitted lifetime detail has no time/group attribution.\n",
            );
        }
        text.push_str("Tokens are provider-reported, not billed cost. Completion is attempt outcome, not voyage success.\n");
        if query.detail.is_some() {
            text.push_str(&display_safe(
                &crate::inference::history::display_json(
                    &serde_json::json!({"group":query.detail,"attempts":data.attempts}),
                )
                .unwrap_or_else(|_| "Historical display unavailable".into()),
            ));
        } else {
            for (index, group) in data.groups.iter().enumerate() {
                text.push_str(&format!(
                    "{} {} · {} permits · input {} · output {}\n",
                    if index == self.selected { "▶" } else { " " },
                    display_safe(
                        &crate::inference::history::display_json(&group.key)
                            .unwrap_or_default()
                            .replace('\n', " ")
                    ),
                    group.totals.retained_attempts,
                    tokens(&group.totals.input),
                    tokens(&group.totals.output)
                ));
            }
            if data.groups.is_empty() {
                text.push_str("No retained attempts match this range.\n");
            }
        }
        text
    }
    pub fn draw(&self, frame: &mut ratatui::Frame<'_>, area: Rect) {
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(" Usage history · Esc close/cancel ")
            .borders(Borders::ALL);
        let inside = block.inner(area);
        frame.render_widget(block, area);
        let lines = publication::lines(&self.text(), inside.width as usize);
        let start = self
            .scroll
            .min(lines.len().saturating_sub(inside.height as usize));
        frame.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(start)
                    .take(inside.height as usize)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            inside,
        );
    }
}
fn tokens(value: &Tokens) -> String {
    format!(
        "{} reported ({} known / {} missing)",
        value
            .reported_sum
            .map_or_else(|| "unavailable".into(), |sum| sum.to_string()),
        value.reported_attempts,
        value.missing_attempts
    )
}
pub(super) fn start(
    app: &mut App,
    agent: Arc<Agent>,
    tx: mpsc::UnboundedSender<UiEvent>,
    query: Query,
    project_scope: bool,
) {
    app.usage_panel.close();
    let session = app.session.id;
    let request = Uuid::new_v4();
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let worker_query = query.clone();
    let authority = agent.clone();
    let task = tokio::spawn(async move {
        let result = agent
            .inference_history(session, project_scope, worker_query, token)
            .await
            .map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::InferenceHistory {
            session,
            request,
            result,
        });
    });
    app.usage_panel = Panel {
        open: true,
        authority: Some(authority),
        session: Some(session),
        request: Some(request),
        job: Some(TitleJob { task, cancel }),
        query: Some(query),
        project_scope,
        ..Default::default()
    };
}
