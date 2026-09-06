//! Exact publication HTTP, approval and durable receipt boundaries.
use super::{publication::{Action,Draft,InlineComment,ReviewEvent,Side},repository::Object,service::Service,store::{State,Store}};
use crate::{Config,config::AccessMode,policy::Policy,tools::{ApprovalOutcome,ApprovalRequest,Approver,InteractionMode,Redactor,ToolContext}};
use serde_json::{Value,json};
use std::{sync::{Arc,Mutex},time::Duration};
use tokio::io::{AsyncReadExt,AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TOKEN: &str = "synthetic-review-fixture-token";
const URL: &str = "https://github.com/o/r/pull/1";
#[derive(Clone,Copy)]
enum Reply { Success,Forbidden,RateLimited,Malformed,Oversized,Interrupted }
struct Approval { calls:Arc<Mutex<Vec<ApprovalRequest>>>, expected:Value }
#[async_trait::async_trait]
impl Approver for Approval {
    async fn approve(&self,request:&ApprovalRequest)->ApprovalOutcome {
        assert_eq!(request.action,"github.publish");
        let preview:Value=serde_json::from_str(&request.reason).unwrap();
        assert_eq!(preview["request"],self.expected);
        assert_eq!(preview["actor"]["id"],7);
        assert_eq!(preview["object"],URL);
        self.calls.lock().unwrap().push(request.clone());
        ApprovalOutcome::Approved
    }
}
fn review(event:ReviewEvent,invalid:u8)->Draft {
    Draft { object:Object::parse(URL).unwrap(),action:Action::Review {
        event,commit_id:HEAD.into(),body:if event==ReviewEvent::Approve { String::new() } else {"Exact review 🧭".into()},
        comments:vec![InlineComment { path:if invalid==2 {"missing.rs".into()} else {"src/a.rs".into()},
            line:if invalid==1 {99} else {2},side:if invalid==3 {Side::Left} else {Side::Right},start_line:None,start_side:None,body:"Exact changed line".into() }],
    }}
}

#[tokio::test]
async fn all_review_events_publish_exact_inline_payload_and_persist_api_receipt() {
    for event in [ReviewEvent::Comment,ReviewEvent::Approve,ReviewEvent::RequestChanges] {
        exercise(review(event,0),Reply::Success,false).await;
    }
}
#[tokio::test]
async fn invalid_inline_locations_never_prepare_approve_or_publish() {
    for invalid in [1,2,3] { exercise(review(ReviewEvent::Comment,invalid),Reply::Success,true).await; }
}
#[tokio::test]
async fn failed_comment_publication_remains_uncertain_without_retransmission() {
    for reply in [Reply::Forbidden,Reply::RateLimited,Reply::Malformed,Reply::Oversized,Reply::Interrupted] {
        exercise(Draft { object:Object::parse(URL).unwrap(),action:Action::Comment { body:"Exact issue-style comment".into() } },reply,false).await;
    }
}

async fn exercise(draft:Draft,reply:Reply,invalid:bool) {
    let directory=tempfile::tempdir().unwrap();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin=reqwest::Url::parse(&format!("http://{}/",listener.local_addr().unwrap())).unwrap();
    let routes=Arc::new(Mutex::new(Vec::<String>::new()));
    let captured=routes.clone();
    let posts=Arc::new(Mutex::new(Vec::<Value>::new()));
    let sent=posts.clone();
    let expected=draft.body();
    let expected_request=expected.clone();
    let action=draft.action.clone();
    let stop=CancellationToken::new();
    let server_stop=stop.clone();
    let server=tokio::spawn(async move {
        loop {
            let (mut socket,_)=tokio::select! {biased; _=server_stop.cancelled()=>break, result=listener.accept()=>result.unwrap()};
            let mut bytes=Vec::new();
            let end=loop {
                let mut chunk=[0;4096];
                let n=tokio::time::timeout(Duration::from_secs(5),socket.read(&mut chunk)).await.unwrap().unwrap();
                assert!(n>0);bytes.extend_from_slice(&chunk[..n]);assert!(bytes.len()<=128*1024);
                if let Some(end)=bytes.windows(4).position(|p|p==b"\r\n\r\n") {break end+4;}
            };
            let headers=String::from_utf8(bytes[..end].to_vec()).unwrap();
            assert!(headers.to_ascii_lowercase().contains(&format!("authorization: bearer {TOKEN}")));
            let route=headers.lines().next().unwrap().to_owned();
            captured.lock().unwrap().push(route.clone());
            let length=headers.lines().find_map(|line| {let(k,v)=line.split_once(':')?;k.eq_ignore_ascii_case("content-length").then(||v.trim().parse::<usize>().unwrap())}).unwrap_or(0);
            assert!(end+length<=128*1024);
            while bytes.len()<end+length {
                let mut chunk=[0;4096];let n=tokio::time::timeout(Duration::from_secs(5),socket.read(&mut chunk)).await.unwrap().unwrap();
                assert!(n>0);bytes.extend_from_slice(&chunk[..n]);assert!(bytes.len()<=128*1024);
            }
            let (status,response)=if route=="GET /user HTTP/1.1" {(200,json!({"id":7,"login":"fixture"}))}
            else if route=="GET /repos/o/r/pulls/1 HTTP/1.1" {(200,json!({"number":1,"html_url":URL,"state":"open","head":{"sha":HEAD},"base":{"sha":HEAD,"ref":"main","repo":{"id":1}}}))}
            else if route=="GET /repos/o/r/pulls/1/files?per_page=100&page=1 HTTP/1.1" {
                // Old side has only line1; new side has line1 and added line2.
                (200,json!([{"filename":"src/a.rs","patch":"@@ -1 +1,2 @@\n context\n+added"}]))
            } else {
                let is_review=matches!(action,Action::Review {..});
                assert_eq!(route,if is_review {"POST /repos/o/r/pulls/1/reviews HTTP/1.1"} else {"POST /repos/o/r/issues/1/comments HTTP/1.1"});
                let payload:Value=serde_json::from_slice(&bytes[end..end+length]).unwrap();
                assert_eq!(payload,expected_request);sent.lock().unwrap().push(payload);
                let (status,value)=match &action {
                    Action::Review {event,commit_id,body,..} => (200,json!({"id":9,"html_url":format!("{URL}#pullrequestreview-9"),"user":{"id":7},"body":body,"commit_id":commit_id,"state":match event {ReviewEvent::Comment=>"COMMENTED",ReviewEvent::Approve=>"APPROVED",ReviewEvent::RequestChanges=>"CHANGES_REQUESTED"}})),
                    Action::Comment {body} => (201,json!({"id":9,"html_url":format!("{URL}#issuecomment-9"),"user":{"id":7},"body":body})),
                };
                match reply {
                    Reply::Success=>(status,value),
                    Reply::Forbidden=>(403,json!({"message":"remote diagnostic must remain private"})),
                    Reply::RateLimited=>(429,json!({"message":"remote diagnostic must remain private"})),
                    Reply::Malformed=> {socket.write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{").await.unwrap();continue;},
                    Reply::Oversized=> {socket.write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n").await.unwrap();continue;},
                    Reply::Interrupted=> {socket.write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{").await.unwrap();continue;},
                }
            };
            let response=serde_json::to_vec(&response).unwrap();
            socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",response.len()).as_bytes()).await.unwrap();
            socket.write_all(&response).await.unwrap();
        }
    });
    let approvals=Arc::new(Mutex::new(vec![]));
    let config=Config {github_enabled:true,access:Some(AccessMode::Unrestricted),..Default::default()};
    let context=ToolContext { github:Some(super::Credential::fixture(TOKEN)),completion:None,
        policy:Arc::new(Policy::new(&config,directory.path().into()).unwrap()),
        approver:Arc::new(Approval {calls:approvals.clone(),expected}),timeout:Duration::from_secs(5),max_output_bytes:64*1024,
        environment:Default::default(),cancellation:CancellationToken::new(),execution_id:Uuid::new_v4(),interaction:InteractionMode::Attended,redactor:Arc::new(Redactor::default()) };
    let path=directory.path().join("operations");let session=Uuid::new_v4();
    let service=Service::fixture(context.clone(),Some(session),origin.clone(),path.clone()).unwrap();
    let operation=tokio::time::timeout(Duration::from_secs(5),service.prepare(draft)).await.unwrap();
    if invalid {
        assert!(operation.is_err(),"invalid inline coordinate prepared successfully");
        assert!(approvals.lock().unwrap().is_empty());assert!(posts.lock().unwrap().is_empty());
        assert!(service.list(0).await.unwrap().is_empty(),"invalid preparation persisted a draft");
    } else {
        let operation=operation.unwrap();
        assert_eq!(operation.state,State::Prepared);
        let result=tokio::time::timeout(Duration::from_secs(5),service.publish(operation.id,&operation.digest)).await.unwrap();
        assert_eq!(approvals.lock().unwrap().len(),1);assert_eq!(posts.lock().unwrap().len(),1);
        let owner=service.owner().clone();drop(service);
        let stored=Store::open(path.clone()).unwrap().inspect(operation.id,&owner).unwrap();
        assert_eq!(stored.digest,operation.digest);
        if matches!(reply,Reply::Success) {
            let published=result.unwrap();assert_eq!(published.state,State::Published);assert_eq!(stored.state,State::Published);
            let receipt=stored.receipt.as_ref().unwrap();assert_eq!(receipt.id,9);
            assert_eq!(receipt.url,format!("{URL}#pullrequestreview-9"));
            assert_eq!(serde_json::to_value(&receipt.evidence).unwrap(),"api_response");
        } else {
            let error=result.unwrap_err().to_string();assert!(!error.contains("remote diagnostic must remain private"));
            assert_eq!(stored.state,State::Sending);assert!(stored.receipt.is_none());
        }
        let before=routes.lock().unwrap().len();
        let recovered=Service::fixture(context,Some(session),origin,path.clone()).unwrap();
        assert!(recovered.publish(operation.id,&operation.digest).await.is_err());
        assert_eq!(routes.lock().unwrap().len(),before,"restart retransmitted or revalidated a terminal/uncertain send");
        assert_eq!(posts.lock().unwrap().len(),1);assert_eq!(approvals.lock().unwrap().len(),1);
        assert_eq!(serde_json::to_value(Store::open(path).unwrap().inspect(operation.id,&owner).unwrap()).unwrap(),serde_json::to_value(stored).unwrap());
    }
    stop.cancel();tokio::time::timeout(Duration::from_secs(5),server).await.unwrap().unwrap();
}
