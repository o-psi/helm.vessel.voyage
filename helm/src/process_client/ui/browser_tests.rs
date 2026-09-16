use super::*;

#[tokio::test]
async fn browser_commands_without_a_local_resource_do_not_start_one_implicitly() {
    let (_fixture, mut app, target) = super::super::coverage_support::app();
    app.clients.mark_unavailable(target.route);
    app.browser_command(target, "/browser status").unwrap();
    assert!(
        app.views[&target]
            .panel
            .as_ref()
            .unwrap()
            .contains("No local browser shared")
    );
    assert!(
        app.views[&target]
            .panel
            .as_ref()
            .unwrap()
            .contains("consent")
    );
    for command in ["takeover", "private", "invalid", "return"] {
        assert!(
            app.browser_command(target, &format!("/browser {command}"))
                .is_err()
        );
        assert!(app.browsers.is_empty());
        assert!(app.browser_retired.is_empty());
    }
    app.browser_command(target, "/browser close").unwrap();
    assert!(app.status.contains("No Voyage cancellation"));
    app.poll_browsers();
    assert!(app.finish_browsers().await.is_ok());
    app.selected = None;
    assert!(app.browser_command(target, "/browser open").is_err());
}

#[tokio::test]
async fn cleanup_consumes_retired_jobs_and_reports_unresolved_errors() {
    let (_fixture, mut app, _) = super::super::coverage_support::app();
    app.browser_retired.push(tokio::spawn(async { Ok(()) }));
    app.browser_retired.push(tokio::spawn(async {
        Err("synthetic unresolved cleanup".to_string())
    }));
    let error = app.finish_browsers().await.unwrap_err();
    assert!(error.to_string().contains("unresolved"));
    assert!(app.browser_retired.is_empty());
    assert!(app.browsers.is_empty());
}
