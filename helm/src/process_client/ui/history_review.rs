//! Honest, read-only context-operation reviews. No speculative backend counts.

pub(super) fn compact(input: &str) -> String {
    let keep = input
        .trim()
        .strip_prefix("KEEP ")
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|n| (1..=100_000).contains(n));
    let boundary = keep.map_or_else(
        || "Choose N below.".to_owned(),
        |n| format!("Requested boundary: newest {n} non-system messages excluded from this reduction pass."),
    );
    format!(
        "Reduce older working context — Keep N\n\n{boundary}\nThis is extractive reduction, NOT a generated summary and NOT deletion down to N messages. Older user text (including task and steering) stays in provider context. Eligible older assistant/tool text may become excerpts and canonical references; complete tool groups stay valid. Prior reductions are not expanded by raising N.\n\nFull canonical history remains saved/readable; no filesystem changes. Reduction persists across restart and affects subsequent provider requests. Your unsent draft is preserved. Exact eligible/reduced counts are not available in this preview; the receipt reports actual reduction (which may be zero).\n\nType KEEP followed by a number (1–100000), for example KEEP 128:"
    )
}

/// Frozen review: never substitute a newer snapshot or a different route on confirm.
#[derive(Clone)]
pub(super) struct BranchReview {
    pub target: super::state::Target,
    pub incarnation: uuid::Uuid,
    pub revision: u64,
    pub branch_id: uuid::Uuid,
    pub host: String,
    pub total: usize,
    pub points: Vec<(usize, String)>,
    pub selected: Option<usize>,
}
impl BranchReview {
    pub fn through_message(&self) -> Option<u64> {
        self.selected.map(|n| self.points[n].0 as u64)
    }
    pub fn move_point(&mut self, previous: bool) {
        // Full history is a distinct choice, never an implicit historical point.
        self.selected = match (self.selected, previous) {
            (None, true) => self.points.len().checked_sub(1),
            (None, false) => (!self.points.is_empty()).then_some(0),
            (Some(n), true) => n.checked_sub(1),
            (Some(n), false) => (n + 1 < self.points.len()).then_some(n + 1),
        };
    }
    pub fn text(&self) -> String {
        let boundary = self.selected.map_or_else(
            || format!("Full saved history: {} messages", self.total),
            |n| format!("Through saved user message #{} (inclusive, {} retained / {} omitted).\nSelected text (bounded preview): {}", self.points[n].0 + 1, self.points[n].0 + 1, self.total.saturating_sub(self.points[n].0 + 1), self.points[n].1));
        format!(
            "Branch conversation\nHost (source and destination): {}\nSource voyage: {}\nNew voyage: {}\nReviewed revision: {}\n\n{boundary}\n\nCtrl+Up/Down: choose loaded saved user boundary or full history. Load older transcript pages before opening this review to choose earlier points.\n\nCreates a separate voyage identity and context; original canonical history is not rewritten. The owner validates complete tool groups and rejects stale revisions. Historical context ends at the selected user message, not the latest reply. No provider execution is started by branching.\n\nNo files are rolled back, copied or isolated; same workspace and host. Later edits may affect shared files. Unsent draft is not copied into context.\n\nOptional name; Enter confirms, Esc cancels:",
            self.host, self.target.session, self.branch_id, self.revision
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keep_n_explains_old_user_text_and_no_generated_summary() {
        let text = compact("KEEP 128");
        for required in [
            "newest 128 non-system",
            "NOT a generated summary",
            "Older user text",
            "Prior reductions are not expanded",
            "actual reduction (which may be zero)",
            "Full canonical history remains",
        ] {
            assert!(text.contains(required), "{required}");
        }
        assert!(compact("KEEP 0").contains("Choose N below"));
        assert!(compact("KEEP 100001").contains("Choose N below"));
    }
    #[test]
    fn historical_selection_is_inclusive_and_full_is_explicit() {
        let mut review = BranchReview {
            target: super::super::state::Target {
                route: super::super::state::Route {
                    id: uuid::Uuid::nil(),
                    generation: 0,
                },
                session: uuid::Uuid::nil(),
            },
            incarnation: uuid::Uuid::nil(),
            revision: 12,
            branch_id: uuid::Uuid::new_v4(),
            host: "test-host".into(),
            total: 10,
            points: vec![(2, "earlier".into()), (7, "later".into())],
            selected: None,
        };
        review.move_point(false);
        assert_eq!(review.through_message(), Some(2));
        assert!(review.text().contains("3 retained / 7 omitted"));
        review.move_point(false);
        assert_eq!(review.through_message(), Some(7));
        review.move_point(false);
        assert_eq!(review.through_message(), None);
        review.move_point(true);
        assert_eq!(review.through_message(), Some(7));
        assert!(
            review
                .text()
                .contains("No files are rolled back, copied or isolated")
        );
    }
}
