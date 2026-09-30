//! Presentation-only execution boundaries: no receipt storage, dispatch or host environment writes.
use super::super::coverage_support;
use uuid::Uuid;
#[test]
fn stale_execution_reply_cannot_replace_new_owner_review_or_input(){
    let (_fixture,mut app,target)=coverage_support::app();
    let pending=[Uuid::new_v4(),Uuid::new_v4(),target.session];
    {let view=app.views.get_mut(&target).unwrap();view.execution_pending=Some(pending);view.execution_uncertain=true;view.panel=Some("current owner review".into());}
    app.execution_arrived(target,Uuid::new_v4(),Err("old owner transport failure".into()));
    let view=&app.views[&target];assert_eq!(view.execution_pending,Some(pending));assert!(view.execution_uncertain);assert_eq!(view.panel.as_deref(),Some("current owner review"));assert!(view.error.is_none());assert_eq!(view.draft.text,"preserved draft");
}
#[test]
fn lost_current_reply_retains_operation_identity_and_fences_approval(){
    let (_fixture,mut app,target)=coverage_support::app();let incarnation=app.views[&target].process.incarnation;
    let pending=[Uuid::new_v4(),Uuid::new_v4(),target.session];app.views.get_mut(&target).unwrap().execution_pending=Some(pending);
    app.execution_arrived(target,incarnation,Err("\u{1b}[31mlost response\u{1b}[0m".into()));
    let view=&app.views[&target];assert!(view.execution_uncertain);assert_eq!(view.execution_pending,Some(pending));assert!(!view.error.as_ref().unwrap().contains('\u{1b}'));assert!(view.execution_review.is_none());assert_eq!(view.draft.text,"preserved draft");assert!(app.status.contains("never repeats approval"));
}
#[test]
fn unresolved_preparation_observation_preserves_draft_and_retained_ids(){
    let (_fixture,mut app,target)=coverage_support::app();let incarnation=app.views[&target].process.incarnation;
    let pending=[Uuid::new_v4(),Uuid::new_v4(),target.session];app.views.get_mut(&target).unwrap().execution_pending=Some(pending);
    app.execution_arrived(target,incarnation,Ok(serde_json::json!({"preparation":{"review_id":pending[0],"command_id":pending[1],"session_id":target.session,"phase":"unconfirmed_preparation","source_cleanup_observed":false}})));
    let view=&app.views[&target];assert_eq!(view.execution_pending,Some(pending));assert!(view.execution_review.is_none());assert_eq!(view.draft.text,"preserved draft");assert!(view.panel.as_ref().unwrap().contains("unconfirmed_preparation"));assert!(view.panel.as_ref().unwrap().contains("does not mean the process is live"));
}
