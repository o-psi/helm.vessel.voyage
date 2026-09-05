use super::*;
use crate::{model::Message, session::Session};
use voyage_protocol::attachment::{Command, Operation, VERSION};

fn binding() -> RemoteBinding {
    RemoteBinding {
        origin: "http://127.0.0.1:9480".into(),
        machine_id: Uuid::new_v4(),
        owner_id: Uuid::new_v4(),
        epoch: 1,
        local_installation_id: Uuid::new_v4(),
        local_principal_id: Uuid::new_v4(),
    }
}
fn fixture() -> (tempfile::TempDir, Journal, Session, RemoteBinding) {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(dir.path().join("journal")).unwrap();
    let session = Session::new("fixture-model".into(), dir.path().to_owned());
    (dir, journal, session, binding())
}

#[test]
fn dedicated_remote_creation_is_atomic_bound_and_never_adopts_private_history() {
    let (_dir, mut journal, mut private, binding) = fixture();
    private.messages.push(Message::new(Role::User, "PRIVATE_CANARY"));
    journal.create_session(&private).unwrap();
    assert!(journal.create_remote_session(&private, &binding).is_err());
    assert!(journal.remote_session(&binding).unwrap().is_none());
    let public = Session::new("fixture-model".into(), private.workspace.clone());
    journal.create_remote_session(&public, &binding).unwrap();
    assert_eq!(journal.remote_session(&binding).unwrap(), Some(public.id));
    let second = Session::new("fixture-model".into(), private.workspace.clone());
    assert!(journal.create_remote_session(&second, &binding).is_err());
    assert_eq!(journal.load_session(private.id).unwrap().session.messages[0].content, "PRIVATE_CANARY");
    let mut changed = binding.clone(); changed.epoch += 1;
    assert!(journal.remote_session(&changed).is_err());
    assert!(journal.remote_replay(&binding, private.id, 0, 100).is_err());
}

#[test]
fn public_replay_has_historical_revision_usage_tool_ids_and_no_private_payload() {
    let (_dir, mut journal, session, binding) = fixture();
    journal.create_remote_session(&session, &binding).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let request = TurnAdmission {command_id:Uuid::new_v4(),machine_id:binding.machine_id,principal_id:binding.owner_id,session_id:session.id,expected_revision:0,expires_at_ms:60000,prompt:"visible task".into()};
    let admitted = journal.admit_turn(&guard, &request, 1).unwrap();
    journal.mark_running(&guard, admitted.run.id).unwrap();
    let mut messages = journal.load_session(session.id).unwrap().session.messages;
    let mut tool = Message::new(Role::Assistant, "");
    tool.tool_calls.push(crate::model::ToolCall{id:"native-non-uuid-id".into(),name:"read_file".into(),arguments:serde_json::json!({"path":"ARGUMENT_CANARY"})});
    tool.provider_state=Some(serde_json::json!({"opaque":"PROVIDER_STATE_CANARY"}));
    messages.push(tool);
    journal.checkpoint_canonical(&guard,admitted.run.id,&messages,&Usage{input_tokens:3,output_tokens:2}).unwrap();
    messages.push(Message::tool("native-non-uuid-id", "RAW_RESULT_CANARY"));
    journal.checkpoint_canonical(&guard,admitted.run.id,&messages,&Usage{input_tokens:3,output_tokens:2}).unwrap();
    let replay=journal.remote_replay(&binding,session.id,0,100).unwrap();
    let RemoteReplay::Events{events,..}=replay else{panic!("expected complete replay")};
    assert!(matches!(events[0].event, RunEvent::Accepted{revision:1,..}));
    let started=events.iter().find_map(|e| match e.event{RunEvent::ToolStarted{tool_call_id,..}=>Some(tool_call_id),_=>None}).unwrap();
    assert!(events.iter().any(|e|matches!(e.event,RunEvent::ToolFinished{tool_call_id,outcome:ToolOutcome::Succeeded} if tool_call_id==started)));
    assert!(events.iter().any(|e|matches!(e.event,RunEvent::Usage{input_tokens:3,output_tokens:2})));
    for (index,event) in events.iter().enumerate(){assert_eq!(event.cursor.get(), index as u64+1);}
    let encoded=serde_json::to_string(&events).unwrap();
    for canary in ["ARGUMENT_CANARY","PROVIDER_STATE_CANARY","RAW_RESULT_CANARY","native-non-uuid-id"]{assert!(!encoded.contains(canary));}
    assert!(journal.remote_replay(&binding,session.id,0,0).is_err());
}

#[test]
fn remote_cancel_receipt_is_atomic_exact_and_never_retargets_after_reconnect() {
    let (_dir,mut journal,session,binding)=fixture();
    journal.create_remote_session(&session,&binding).unwrap();
    let guard=journal.acquire_execution(session.id).unwrap();
    let request=TurnAdmission{command_id:Uuid::new_v4(),machine_id:binding.machine_id,principal_id:binding.owner_id,session_id:session.id,expected_revision:0,expires_at_ms:60000,prompt:"task".into()};
    let run=journal.admit_turn(&guard,&request,1).unwrap().run;
    let mut cancel=Command{version:VERSION,connection_id:Uuid::new_v4(),machine_id:binding.machine_id,principal_id:binding.owner_id,command_id:Uuid::new_v4(),expires_at_ms:60000,operation:Operation::Cancel{session_id:session.id,run_id:run.id}};
    assert!(!journal.remote_cancel(&binding,&cancel,2).unwrap().duplicate);
    assert!(journal.local_cancel_requested(session.id,run.id).unwrap());
    cancel.connection_id=Uuid::new_v4();
    assert!(journal.remote_cancel(&binding,&cancel,70000).unwrap().duplicate);
    cancel.expires_at_ms+=1;
    assert!(journal.remote_cancel(&binding,&cancel,2).is_err());
    cancel.command_id=request.command_id;
    assert!(journal.remote_cancel(&binding,&cancel,2).is_err());
    assert!(journal.mark_running(&guard,run.id).is_err());
}
