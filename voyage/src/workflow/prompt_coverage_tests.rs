use super::*;
#[test]
fn cancellation_and_restoration_without_a_terminal_are_idempotent() {
    let control = Control(Arc::new(Mutex::new(State {
        terminal: None,
        cancelled: false,
    })));
    assert!(control.read().is_err());
    assert!(control.discard().is_err());
    control.finish(false).unwrap();
    assert!(!control.0.lock().unwrap().cancelled);
    drop(Restore(control.clone()));
    assert!(!control.0.lock().unwrap().cancelled);
    drop(CancelOnDrop(control.clone()));
    assert!(control.0.lock().unwrap().cancelled);
    control.finish(false).unwrap();
    control.finish(true).unwrap();
    assert!(control.start().is_err());
    assert!(control.read().is_err());
}
