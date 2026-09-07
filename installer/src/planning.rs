//! One read-only planner keeps hashing and service inspection outside terminal input.
use crate::{cli::Options, flow, service};
use std::sync::mpsc::{self, Receiver};

pub fn start(options: Options) -> Receiver<Result<Vec<String>, String>> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<Vec<String>> {
            let report = flow::plan(&options)?;
            let mut lines = flow::describe(&report, &options);
            lines.push(service::preview(
                &report.release_dir.join("bin"),
                options.start,
            )?);
            Ok(lines)
        })()
        .map_err(|error| format!("{error:#}"));
        let _ = send.send(result);
    });
    receive
}
