//! Owned preparation worker: cancellation joins before its staging can be discarded.
use crate::{cli::Options, flow, service};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
};

pub type Review = (Options, Vec<String>);
pub struct Task {
    pub receive: Receiver<Result<Review, String>>,
    cancelled: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Task {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
pub fn start(mut options: Options) -> Task {
    let (send, receive) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    #[cfg(all(test, target_os = "linux"))]
    let fixture = crate::fixture_tests::capture();
    let worker = std::thread::spawn(move || {
        #[cfg(all(test, target_os = "linux"))]
        crate::fixture_tests::enter(fixture);
        let result = (|| -> anyhow::Result<Review> {
            options.prepare(&flag)?;
            anyhow::ensure!(!flag.load(Ordering::Relaxed), "Preparation cancelled");
            let report = flow::plan(&options)?;
            let mut lines = flow::describe(&report, &options);
            lines.push(service::preview(
                &report.release_dir.join("bin"),
                options.start,
            )?);
            Ok((options, lines))
        })()
        .map_err(|error| format!("{error:#}"));
        let _ = send.send(result);
    });
    Task {
        receive,
        cancelled,
        worker: Some(worker),
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "planning_tests.rs"]
mod tests;
