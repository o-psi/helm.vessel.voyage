use super::*;
use crate::process_client::connections::{Connection, Metadata};
fn connection() -> Connection {
    let id = Uuid::new_v4();
    Connection {
        id: Uuid::new_v4(),
        alias: "remote fixture".into(),
        endpoint: "https://example.invalid".into(),
        vessel_id: Uuid::new_v4(),
        principal_id: Some(Uuid::new_v4()),
        grant_id: Uuid::new_v4(),
        scope: Scope::Workspaces {
            workspace_ids: vec![id],
        },
        credential_ref: Uuid::new_v4(),
        autoconnect: false,
        workspace_preference: None,
        revision: 1,
        forgotten: false,
        legacy_route: None,
        metadata: Metadata {
            features: vec!["workspace_pairing".into()],
            rights: vec!["create".into()],
            expires_at_ms: Some(u64::MAX),
            workspaces: vec![Workspace {
                id,
                path: "/remote/fixture".into(),
                name: "fixture".into(),
                provider_ready: Some(true),
            }],
            ..Default::default()
        },
    }
}
#[test]
fn authorization_is_exact_expiring_and_never_infers_arbitrary_remote_paths() {
    let original = connection();
    let client = Client::from_connection(original.clone(), "/not-read".into());
    let choices = authorized(&client).unwrap();
    assert_eq!(choices.len(), 1);
    assert_eq!(
        select(&choices, "/remote/fixture").unwrap().id,
        choices[0].id
    );
    assert_eq!(
        select(&choices, &choices[0].id.to_string()).unwrap().id,
        choices[0].id
    );
    assert!(select(&choices, "/remote/other").is_err());
    assert!(select(&[choices[0].clone(), choices[0].clone()], "/remote/fixture").is_err());
    for field in [
        "scope",
        "features",
        "rights",
        "expiry",
        "workspaces",
        "path",
    ] {
        let mut c = original.clone();
        match field {
            "scope" => {
                c.scope = Scope::Session {
                    session_id: Uuid::new_v4(),
                }
            }
            "features" => c.metadata.features.clear(),
            "rights" => c.metadata.rights.clear(),
            "expiry" => c.metadata.expires_at_ms = Some(0),
            "workspaces" => c.metadata.workspaces.clear(),
            "path" => c.metadata.workspaces[0].path = "relative".into(),
            _ => unreachable!(),
        };
        assert!(
            authorized(&Client::from_connection(c, "/not-read".into())).is_err(),
            "{field}"
        );
    }
    assert!(authorized(&Client::local("/not-read".into())).is_err());
}
#[test]
fn workspace_picker_navigation_and_rendering_never_changes_draft_on_escape() {
    use crossterm::event::{KeyEvent, KeyModifiers};
    let (_fixture, mut app, target) = crate::process_client::ui::coverage_support::app();
    let first = connection().metadata.workspaces.remove(0);
    let mut second = first.clone();
    second.id = Uuid::new_v4();
    second.path = "/remote/second".into();
    let route = target.route;
    app.workspace_picker = Some(WorkspacePicker::new(
        route,
        vec![first, second.clone()],
        Some(second.id),
    ));
    assert_eq!(app.workspace_picker.as_ref().unwrap().selected, 1);
    for (key, expected) in [
        (KeyCode::Home, 0),
        (KeyCode::Down, 1),
        (KeyCode::Down, 1),
        (KeyCode::Up, 0),
        (KeyCode::End, 1),
    ] {
        app.workspace_picker_input(&Event::Key(KeyEvent::new(key, KeyModifiers::NONE)))
            .unwrap();
        assert_eq!(app.workspace_picker.as_ref().unwrap().selected, expected);
    }
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|f| app.draw_workspace_picker(f, f.area()))
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("second"));
    app.workspace_picker_input(&Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
        .unwrap();
    assert!(app.workspace_picker.is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}
