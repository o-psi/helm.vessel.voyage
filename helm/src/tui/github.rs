//! Session-scoped operator jobs and bounded, scrollable GitHub observations.
use super::{App, ApprovalRequest, TitleJob, UiEvent, publication};
use crate::{
    Agent,
    tools::{ApprovalOutcome, Approver},
};
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    pub request: Option<Uuid>,
    pub session: Option<Uuid>,
    pub display: String,
    pub scroll: usize,
    pub authority: Option<Arc<Agent>>,
    job: Option<TitleJob>,
}
impl Panel {
    #[cfg(test)]
    pub(super) fn own_test_job(&mut self, task: tokio::task::JoinHandle<()>, cancel: CancellationToken) {
        self.job = Some(TitleJob { task, cancel });
    }
    pub fn matches(&self, session: Uuid, request: Uuid) -> bool {
        self.open && self.session == Some(session) && self.request == Some(request)
    }
    pub fn close(&mut self) {
        self.job = None;
        self.request = None;
        self.open = false;
    }
    pub fn key(&mut self, key: KeyEvent, area: Rect) {
        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('c')
                && key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL))
        {
            self.close();
            return;
        }
        let maximum = publication::lines(&self.display, area.width.saturating_sub(2) as usize)
            .len()
            .saturating_sub(area.height.saturating_sub(3) as usize);
        publication::navigate(
            key,
            &mut self.scroll,
            maximum,
            area.height.saturating_sub(3) as usize,
        );
    }
    pub fn draw(&self, frame: &mut ratatui::Frame<'_>, area: Rect) {
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(" GitHub · ↑↓ PgUp/PgDn · Esc close/cancel ")
            .borders(Borders::ALL);
        let inside = block.inner(area);
        frame.render_widget(block, area);
        let lines = publication::lines(&self.display, inside.width as usize);
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
    pub fn finished(&mut self, display: String) {
        self.job = None;
        self.request = None;
        self.display = display;
        self.scroll = 0;
    }
}

struct ScopedApprover {
    tx: mpsc::UnboundedSender<UiEvent>,
    session: Uuid,
    request: Uuid,
}
pub(super) fn scoped_approver(tx: mpsc::UnboundedSender<UiEvent>, session: Uuid, request: Uuid) -> Arc<dyn Approver> {
    Arc::new(ScopedApprover { tx, session, request })
}
#[async_trait]
impl Approver for ScopedApprover {
    async fn approve(&self, value: &crate::tools::ApprovalRequest) -> ApprovalOutcome {
        let (response, receive) = oneshot::channel();
        let event = UiEvent::GithubApproval {
            session: self.session,
            request: self.request,
            approval: ApprovalRequest {
                id: value.id,
                action: value.action.clone(),
                target: value.target.clone(),
                reason: value.reason.clone(),
                response,
            },
        };
        if self.tx.send(event).is_err() {
            return ApprovalOutcome::Unavailable;
        }
        receive.await.unwrap_or(ApprovalOutcome::Unavailable)
    }
}

pub(super) fn start(
    app: &mut App,
    agent: Arc<Agent>,
    tx: mpsc::UnboundedSender<UiEvent>,
    words: Vec<String>,
) {
    let session = app.session.id;
    let request = Uuid::new_v4();
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let approver = scoped_approver(tx.clone(), session, request);
    let authority = agent.clone();
    let task = tokio::spawn(async move {
        let result = agent
            .github_command_with_approver(session, words, worker_cancel, approver)
            .await
            .map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::GithubResult {
            session,
            request,
            result,
        });
    });
    app.github_panel = Panel { open:true, request:Some(request), session:Some(session), display:"GitHub operation in progress. Esc cancels waiting; a sent publication may remain uncertain. Use /github inspect to recover.".into(), scroll:0, authority:Some(authority), job:Some(TitleJob {task,cancel}) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closing_panel_cancels_owned_job_and_invalidates_approval_scope() {
        let session = Uuid::new_v4();
        let request = Uuid::new_v4();
        let cancel = CancellationToken::new();
        let task = tokio::spawn(std::future::pending());
        let mut panel = Panel {
            open: true,
            session: Some(session),
            request: Some(request),
            job: Some(TitleJob { task, cancel: cancel.clone() }),
            ..Default::default()
        };
        assert!(panel.matches(session, request));
        panel.key(KeyEvent::from(KeyCode::Esc), Rect::new(0, 0, 40, 12));
        assert!(cancel.is_cancelled());
        assert!(!panel.matches(session, request));
        assert!(panel.job.is_none());
    }
}
