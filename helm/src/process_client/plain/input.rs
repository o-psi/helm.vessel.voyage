//! A detached OS reader does not hold Tokio's runtime open after interface exit.
use anyhow::{Result, ensure};
use std::io::{BufRead, Read};
use tokio::sync::mpsc;

pub(super) fn lines() -> mpsc::Receiver<Result<String>> {
    let (sender, receiver) = mpsc::channel(8);
    std::thread::spawn(move || {
        let input = std::io::stdin();
        let mut input = input.lock();
        loop {
            let result = (|| -> Result<Option<String>> {
                let mut bytes = Vec::new();
                let count = input.by_ref().take(65538).read_until(b'\n', &mut bytes)?;
                if count == 0 {
                    return Ok(None);
                }
                ensure!(bytes.len() <= 65537, "line exceeds 64 KiB input limit");
                if bytes.last() == Some(&b'\n') {
                    bytes.pop();
                }
                if bytes.last() == Some(&b'\r') {
                    bytes.pop();
                }
                Ok(Some(String::from_utf8(bytes)?))
            })();
            match result {
                Ok(Some(line)) => {
                    if sender.blocking_send(Ok(line)).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = sender.blocking_send(Err(error));
                    break;
                }
            }
        }
    });
    receiver
}
