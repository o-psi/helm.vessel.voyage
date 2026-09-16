use super::*;
#[test]
fn invalid_preparation_is_reported_and_worker_joined() {
    let mut options = Options::parse(&["install".into()]).unwrap();
    options.dev = true;
    let task = start(options);
    let result = task
        .receive
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    assert!(result.err().unwrap().contains("--dev"));
    drop(task);
}
#[test]
fn dropping_task_sets_cancellation_and_joins_owned_worker() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let completed = Arc::new(AtomicBool::new(false));
    let done = completed.clone();
    let worker = std::thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            std::thread::yield_now();
        }
        done.store(true, Ordering::Relaxed);
    });
    let (_send, receive) = mpsc::channel();
    drop(Task {
        receive,
        cancelled: cancelled.clone(),
        worker: Some(worker),
    });
    assert!(cancelled.load(Ordering::Relaxed));
    assert!(completed.load(Ordering::Relaxed));
}
#[test]
fn dropped_result_receiver_does_not_panic_worker() {
    let mut options = Options::parse(&["install".into()]).unwrap();
    options.dev = true;
    drop(start(options));
}

#[test]
fn preparation_worker_delivers_exact_read_only_review() {
    use crate::fixture_tests::{Fixture, release};
    let f = Fixture::new();
    let bin = release(&f, "source", "1.0");
    let options = Options::parse(&[
        "install".into(),
        "--bin-dir".into(),
        bin.to_str().unwrap().into(),
    ])
    .unwrap();
    f.effective(false);
    f.query("ActiveState", "inactive");
    f.query("UnitFileState", "disabled");
    f.query("InvocationID", "");
    let task = start(options);
    let (options, lines) = task
        .receive
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert_eq!(options.bin_dir, bin);
    assert!(lines.iter().any(|line| line.contains("KillMode=process")));
    assert!(lines.iter().any(|line| line.contains("Install release")));
    drop(task);
    assert!(!f.root.join("install").exists());
    assert!(!f.root.join("units").exists());
}
