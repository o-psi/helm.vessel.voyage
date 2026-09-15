//! Scripted loopback-only HTTP peer. Every expected request is bounded and checked.
use super::transport::Client;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) struct Reply {
    pub method: &'static str,
    pub path: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl Reply {
    pub fn json(path: impl Into<String>, body: Value) -> Self {
        Self {
            method: "GET",
            path: path.into(),
            status: 200,
            headers: vec![],
            body: serde_json::to_vec(&body).unwrap(),
        }
    }
    pub fn post(body: Value) -> Self {
        Self {
            method: "POST",
            ..Self::json("/graphql", body)
        }
    }
}

pub(super) struct Fixture {
    pub address: std::net::SocketAddr,
    task: tokio::task::JoinHandle<Vec<Vec<u8>>>,
}
impl Fixture {
    pub async fn start(replies: Vec<Reply>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            tokio::time::timeout(std::time::Duration::from_secs(10), async move {
                let mut requests = Vec::new();
                for reply in replies {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut bytes = Vec::new();
                    let header_end = loop {
                        let mut chunk = [0; 4096];
                        let count = socket.read(&mut chunk).await.unwrap();
                        assert!(count > 0, "request ended before headers");
                        bytes.extend_from_slice(&chunk[..count]);
                        assert!(bytes.len() < 256 * 1024);
                        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                            break end + 4;
                        }
                    };
                    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                    assert_eq!(
                        headers.lines().next().unwrap(),
                        format!("{} {} HTTP/1.1", reply.method, reply.path)
                    );
                    let length = headers
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                        .unwrap_or(0);
                    while bytes.len() < header_end + length {
                        let mut chunk = [0; 4096];
                        let count = socket.read(&mut chunk).await.unwrap();
                        assert!(count > 0, "request ended before body");
                        bytes.extend_from_slice(&chunk[..count]);
                    }
                    requests.push(bytes);
                    let mut response =
                        format!("HTTP/1.1 {} Fixture\r\nConnection: close\r\n", reply.status);
                    if !reply.headers.iter().any(|(key, _)| {
                        key.eq_ignore_ascii_case("content-length")
                            || key.eq_ignore_ascii_case("transfer-encoding")
                    }) {
                        response.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
                    }
                    for (name, value) in reply.headers {
                        response.push_str(&format!("{name}: {value}\r\n"));
                    }
                    response.push_str("\r\n");
                    // Rejected oversized replies may close before all bytes are sent.
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.write_all(&reply.body).await;
                }
                requests
            })
            .await
            .expect("fixture requests timed out")
        });
        Self { address, task }
    }
    pub fn client(&self) -> Client {
        Client::for_test(self.address)
    }
    pub async fn finish(mut self) -> Vec<Vec<u8>> {
        (&mut self.task).await.unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
