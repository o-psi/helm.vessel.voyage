use super::*;
#[test]
fn assignment_arguments_are_strict_and_defaults_do_not_invent_disclosure() {
    for value in [
        json!({"action":"submit","participant":"local","task":"offline"}),
        json!({"action":"observe","participant":"local","assignment_id":Uuid::nil()}),
        json!({"action":"cancel","participant":"local","assignment_id":Uuid::nil()}),
        json!({"action":"list"}),
    ] {
        assert!(serde_json::from_value::<Args>(value.clone()).is_ok());
        let mut extra = value;
        extra["unexpected"] = json!(true);
        if extra["action"] != "list" {
            assert!(serde_json::from_value::<Args>(extra).is_err());
        }
    }
    let action: Args =
        serde_json::from_value(json!({"action":"submit","participant":"local","task":"offline"}))
            .unwrap();
    match action {
        Args::Submit {
            context,
            assignment_id,
            ..
        } => {
            assert!(context.is_empty());
            assert!(assignment_id.is_none());
        }
        _ => panic!("wrong action"),
    }
    for value in [
        json!({"action":"submit","task":"offline"}),
        json!({"action":"retry","assignment_id":"wrong"}),
        json!({"action":"observe","assignment_id":Uuid::nil()}),
    ] {
        assert!(serde_json::from_value::<Args>(value).is_err());
    }
}
