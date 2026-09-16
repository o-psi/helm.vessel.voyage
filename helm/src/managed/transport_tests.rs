use super::*;
use helm::process_client::transport::Client;
use voyage_protocol::vessel::{ProcessInfo, ProcessState};

#[tokio::test]
async fn managed_owner_state_fences_do_not_connect_or_recover_implicitly() {
    let client = Client::local("/synthetic/not-opened".into());
    for state in [
        ProcessState::Starting,
        ProcessState::Live,
        ProcessState::Suspended,
        ProcessState::Unavailable,
        ProcessState::CleanupUnconfirmed,
        ProcessState::Relinquished,
    ] {
        let process = ProcessInfo {
            session_id: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
            workspace: "/synthetic".into(),
            state: state.clone(),
            name: None,
            catalogue: None,
            archive: None,
            deletion: None,
        };
        let result = connection::live(&client, process.clone()).await;
        assert_eq!(
            result.is_ok(),
            matches!(state, ProcessState::Live | ProcessState::Suspended)
        );
        if let Ok(observed) = result {
            assert_eq!(observed.session_id, process.session_id);
            assert_eq!(observed.incarnation, process.incarnation);
        }
        assert!(client.connection_state().borrow().socket_id.is_none());
    }
    assert!(
        connection::connect(std::path::Path::new("relative"))
            .await
            .is_err()
    );
    for limit in [0, 101, usize::MAX] {
        assert!(
            catalogue::list(&client, std::path::Path::new("/synthetic"), None, limit)
                .await
                .is_err()
        );
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    assert!(connection::deadline().unwrap() >= now + 300_000);
}

#[test]
fn managed_safe_errors_never_retain_control_characters() {
    assert_eq!(
        safe_error(anyhow::anyhow!("x\x1b[31m\u{202e}y\nz")).to_string(),
        "x[31myz"
    );
}
