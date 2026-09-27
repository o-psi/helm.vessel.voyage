use super::*;

#[test]
fn frame_budget_reset_is_shared_with_clones() {
    let budget = Budget::new();
    let clone = budget.clone();
    *budget.deadline.lock().unwrap() = Instant::now() - Duration::from_secs(1);
    let before = Instant::now();
    clone.begin_frame().unwrap();
    let deadline = *budget.deadline.lock().unwrap();
    assert!(deadline >= before + Duration::from_millis(250));
    assert_eq!(deadline, *clone.deadline.lock().unwrap());
}

#[test]
fn poisoned_budget_returns_error_instead_of_panicking_or_writing() {
    let budget = Budget::new();
    let clone = budget.clone();
    assert!(
        std::thread::spawn(move || {
            let _guard = clone.deadline.lock().unwrap();
            panic!("synthetic budget poison");
        })
        .join()
        .is_err()
    );
    let error = budget.begin_frame().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert!(error.to_string().contains("budget unavailable"));
}
