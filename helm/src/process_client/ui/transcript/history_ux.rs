//! Read-only history navigation: never changes canonical text or run identity.
use super::{Key, State};

impl State {
    pub(in crate::process_client::ui) fn branch_points(
        &self,
    ) -> (Vec<(usize, String)>, Option<usize>) {
        let points: Vec<_> = self
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .map(|m| (m.message_index, m.content.chars().take(512).collect()))
            .collect();
        let index = self.anchor.as_ref().and_then(|a| match a.key {
            Key::Message(i) | Key::MessageHeading(i) | Key::Sender(i) => Some(i),
            _ => None,
        });
        let selected = index.and_then(|i| points.iter().position(|(n, _)| *n == i));
        (points, selected)
    }

    pub(in crate::process_client::ui) fn branch_points_for(
        &self,
        snapshot: &super::super::state::Snapshot,
    ) -> (Vec<(usize, String)>, Option<usize>) {
        if self.loaded_revision == Some(snapshot.revision) {
            return self.branch_points();
        }
        // Complete current snapshot messages already have canonical indices.
        // Never borrow a stale hydrated page simply because it is longer.
        let points: Vec<_> = snapshot
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .map(|m| {
                (
                    m.message_index,
                    crate::process_client::safe(&m.content.chars().take(512).collect::<String>()),
                )
            })
            .collect();
        let at = self.anchor.as_ref().and_then(|a| match a.key {
            Key::Message(i) | Key::MessageHeading(i) | Key::Sender(i) => Some(i),
            _ => None,
        });
        let selected = at.and_then(|index| points.iter().position(|(n, _)| *n == index));
        (points, selected)
    }

    pub(super) fn older(&mut self, first: usize) {
        self.requested_from = Some(first.saturating_sub(128));
        if !self.loading {
            self.attempted = None;
            self.live_attempt = None;
        }
        // Loading earlier pages must not move a reader back to the first loaded row.
        if self.anchor.is_none() {
            self.remember(self.top);
        }
    }

    pub(super) fn user_jump(&mut self, users: &[usize], previous: bool) -> bool {
        let current = self
            .anchor
            .as_ref()
            .and_then(|a| {
                self.rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.key == a.key && r.offset <= a.offset)
                    .map(|(index, _)| index)
                    .next_back()
            })
            .unwrap_or(self.top);
        let candidates = self.rows.iter().enumerate().filter(|(_, row)| {
            row.offset == 0
                && !row.line.to_string().trim().is_empty()
                && matches!(row.key, Key::MessageHeading(index) | Key::Sender(index) if users.contains(&index))
        });
        let found = if previous {
            candidates
                .filter(|(index, _)| *index < current)
                .map(|(index, _)| index)
                .next_back()
        } else {
            candidates
                .filter(|(index, _)| *index > current)
                .map(|(index, _)| index)
                .next()
        };
        if let Some(index) = found {
            self.remember(index);
            true
        } else {
            false
        }
    }

    pub(super) fn find_match(&self, top: usize) -> Option<usize> {
        let count = self.rows.len();
        if count == 0 || self.query.is_empty() {
            return None;
        }
        let current = self
            .anchor
            .as_ref()
            .and_then(|a| {
                self.rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.key == a.key && r.offset <= a.offset)
                    .map(|(index, _)| index)
                    .next_back()
            })
            .unwrap_or(top)
            .min(count - 1);
        let query = self.query.to_lowercase();
        (1..=count)
            .map(|distance| {
                if self.search_previous {
                    (current + count - distance) % count
                } else {
                    (current + distance) % count
                }
            })
            .find(|&index| {
                self.rows[index]
                    .line
                    .to_string()
                    .to_lowercase()
                    .contains(&query)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Line;
    fn state() -> State {
        let mut state = State::default();
        state.rows = (0..4)
            .map(|index| super::super::Row {
                key: Key::MessageHeading(index),
                offset: 0,
                line: Line::raw(if index % 2 == 0 { "héllo" } else { "other" }),
            })
            .collect();
        state
    }
    #[test]
    fn complete_current_snapshot_can_branch_without_hydration_or_stale_cache() {
        let snapshot: super::super::super::state::Snapshot = serde_json::from_value(serde_json::json!({
            "session_id":uuid::Uuid::nil(),"revision":3,"model":"fixture","total_messages":2,
            "messages":[{"role":"user","content":"current","message_index":0},{"role":"assistant","content":"reply","message_index":1}]
        })).unwrap();
        let mut state = State::default();
        state.loaded_revision = Some(2);
        state.messages = vec![
            serde_json::from_value(
                serde_json::json!({"role":"user","content":"stale","message_index":99}),
            )
            .unwrap(),
        ];
        let (points, selected) = state.branch_points_for(&snapshot);
        assert_eq!(points, vec![(0, "current".into())]);
        assert_eq!(selected, None);
    }
    #[test]
    fn match_direction_wraps_and_uses_anchor_not_clamped_viewport() {
        let mut state = state();
        state.query = "HÉLLO".into();
        state.remember(2);
        assert_eq!(state.find_match(0), Some(0));
        state.search_previous = true;
        assert_eq!(state.find_match(0), Some(0));
        state.remember(0);
        assert_eq!(state.find_match(0), Some(2));
        state.query = "absent".into();
        assert_eq!(state.find_match(0), None);
    }
    #[test]
    fn user_jumps_skip_non_user_and_do_not_wrap() {
        let mut state = state();
        state.remember(0);
        assert!(state.user_jump(&[0, 3], false));
        assert_eq!(state.top, 3);
        assert!(!state.user_jump(&[0, 3], false));
        assert!(state.user_jump(&[0, 3], true));
        assert_eq!(state.top, 0);
        assert!(!state.user_jump(&[0, 3], true));
    }
    #[test]
    fn user_jump_ignores_heading_gap_and_accepts_coordinated_user() {
        let mut state = state();
        state.rows[1].line = Line::raw("");
        state.rows[2].key = Key::Sender(2);
        state.remember(0);
        assert!(state.user_jump(&[1, 2], false));
        assert_eq!(state.top, 2);
    }
    #[test]
    fn search_resolves_anchor_after_rewrap_and_older_prepend() {
        let mut state = state();
        state.rows[2].offset = 12;
        state.remember(2);
        state.rows[2].offset = 10; // Same content now wraps at a different offset.
        state.rows.insert(
            0,
            super::super::Row {
                key: Key::MessageHeading(99),
                offset: 0,
                line: Line::raw("héllo"),
            },
        );
        state.query = "héllo".into();
        assert_eq!(state.find_match(0), Some(0));
        state.search_previous = true;
        assert_eq!(state.find_match(0), Some(1));
    }
    #[test]
    fn older_fetch_preserves_read_anchor_and_allows_retry() {
        let mut state = state();
        state.remember(2);
        state.attempted = Some(8);
        state.older(200);
        assert_eq!(state.requested_from, Some(72));
        assert_eq!(state.anchor.as_ref().unwrap().key, Key::MessageHeading(2));
        assert_eq!(state.attempted, None);
        state.older(0);
        assert_eq!(state.requested_from, Some(0));
    }
}

#[cfg(test)]
mod branch_point_tests {
    use super::*;
    #[test]
    fn selection_uses_canonical_saved_index_not_loaded_page_offset() {
        let mut state = State::default();
        state.messages = vec![
            super::super::Message {
                message_index: 50,
                role: "assistant".into(),
                content: "excluded".into(),
                ..Default::default()
            },
            super::super::Message {
                message_index: 51,
                role: "user".into(),
                content: "selected".into(),
                ..Default::default()
            },
        ];
        state.anchor = Some(super::super::Anchor {
            key: Key::Message(51),
            offset: 9,
        });
        let (points, selected) = state.branch_points();
        assert_eq!(points, vec![(51, "selected".into())]);
        assert_eq!(selected, Some(0));
        state.anchor = Some(super::super::Anchor {
            key: Key::Message(50),
            offset: 0,
        });
        assert_eq!(state.branch_points().1, None);
        state.anchor = None;
        assert_eq!(state.branch_points().1, None);
    }
}
