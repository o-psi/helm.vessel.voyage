//! Every runtime decision shares one review surface; questions never grant authority.
mod input;
mod render;
mod response;
mod wrap;
use super::{composer::Composer, state::Target};
use anyhow::{Result, ensure};
pub(super) use render::draw;
use std::collections::BTreeMap;
use uuid::Uuid;

type Identity = (Target, Uuid);
#[derive(Default)]
struct AnswerDraft {
    text: Composer,
    option: Option<usize>,
}
#[derive(Default)]
pub(super) struct Review {
    // Only the last rendered identity can receive a response.
    displayed: Option<Identity>,
    selected: Option<Identity>,
    pub(super) focused: bool,
    scroll: u16,
    answers: BTreeMap<Identity, AnswerDraft>,
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(u64::MAX, |time| {
            time.as_millis().min(u64::MAX as u128) as u64
        })
}
fn validate_response(request: &serde_json::Value, response: &serde_json::Value) -> Result<()> {
    match request["kind"].as_str() {
        Some("approval") => ensure!(
            response == "approved" || response == "denied",
            "invalid approval response"
        ),
        Some("question") => match response["status"].as_str() {
            Some("selected") => {
                let index = response["index"]
                    .as_u64()
                    .and_then(|index| usize::try_from(index).ok());
                ensure!(
                    index.and_then(|index| request["question"]["options"].get(index))
                        == Some(&response["answer"]),
                    "answer does not match a question option"
                );
            }
            Some("custom") => {
                let answer = response["answer"].as_str().unwrap_or("");
                ensure!(
                    !answer.trim().is_empty()
                        && answer.len() <= 4096
                        && !answer.chars().any(char::is_control),
                    "answer must be nonblank, at most 4096 bytes, and contain no control characters"
                );
            }
            Some("cancelled") => {}
            _ => anyhow::bail!("invalid question response"),
        },
        _ => anyhow::bail!("this runtime interaction kind is not supported by this Helm"),
    }
    Ok(())
}
