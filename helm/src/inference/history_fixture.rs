//! Inert deterministic ledger seeder for the production CLI system fixture.
use super::*;
#[test]
fn seed_history() {
    let Ok(mode) = std::env::var("HELM_HISTORY_FIXTURE") else {
        return;
    };
    let workspace = PathBuf::from(std::env::var("HELM_HISTORY_WORKSPACE").unwrap());
    let path = Store::default_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut store = Store::open(path).unwrap();
    let project = store.project(&workspace).unwrap();
    if mode == "report" {
        store
            .report(
                std::env::var("HELM_HISTORY_ATTEMPT")
                    .unwrap()
                    .parse()
                    .unwrap(),
                Some(7),
                None,
            )
            .unwrap();
        return;
    }
    if mode == "omitted" {
        let session: Uuid = std::env::var("HELM_HISTORY_SESSION")
            .unwrap()
            .parse()
            .unwrap();
        for scope in [Scope::Project(project), Scope::Session(session)] {
            store
                .connection
                .execute(
                    "UPDATE limits SET consumed=consumed+5,omitted=omitted+5 WHERE scope=?1",
                    [scope.key()],
                )
                .unwrap();
        }
        return;
    }
    assert_eq!(mode, "seed");
    let session = Uuid::new_v4();
    let other = Uuid::new_v4();
    let child = Uuid::new_v4();
    store.bind_session(project, session).unwrap();
    store.bind_session(project, other).unwrap();
    let mut ids = Vec::new();
    for (session, at, model, input, output, outcome, agent, purpose) in [
        (
            session,
            "2026-01-01T00:00:00Z",
            "model α",
            Some(0),
            None,
            AttemptOutcome::Unknown,
            None,
            Purpose::Conversation,
        ),
        (
            session,
            "2026-01-01T12:00:00Z",
            "model β",
            None,
            Some(4),
            AttemptOutcome::Failed,
            Some(child),
            Purpose::Conversation,
        ),
        (
            session,
            "2026-01-02T00:00:00Z",
            "model β",
            Some(999),
            Some(999),
            AttemptOutcome::Completed,
            None,
            Purpose::Title,
        ),
        (
            other,
            "2026-01-01T20:00:00Z",
            "model α",
            Some(3),
            Some(5),
            AttemptOutcome::Completed,
            None,
            Purpose::Conversation,
        ),
    ] {
        let permit = store
            .admit(&Attribution {
                session,
                run: Uuid::new_v4(),
                agent,
                provider: "fixture".into(),
                model: model.into(),
                purpose,
            })
            .unwrap();
        let text: String = store
            .connection
            .query_row(
                "SELECT record FROM attempts WHERE id=?1",
                [permit.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        let mut row: Attempt = serde_json::from_str(&text).unwrap();
        row.admitted_at = at.into();
        row.finished_at = (outcome != AttemptOutcome::Unknown).then(|| at.into());
        row.outcome = outcome;
        row.input_tokens = input;
        row.output_tokens = output;
        store
            .connection
            .execute(
                "UPDATE attempts SET record=?2 WHERE id=?1",
                params![permit.id.to_string(), serde_json::to_string(&row).unwrap()],
            )
            .unwrap();
        ids.push(permit.id);
    }
    println!(
        "HISTORY_SEED={}",
        serde_json::json!({"project":project,"session":session,"other":other,"child":child,"ids":ids})
    );
}
