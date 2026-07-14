//! Shared transfer test fixtures: a tiny controllable HTTP origin.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crunchydl::{RepresentationTransferPlan, SegmentRequest};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

pub struct FixtureServer {
    pub base: String,
    requests: Arc<Mutex<HashMap<String, usize>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl FixtureServer {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture server binds");
        listener
            .set_nonblocking(true)
            .expect("listener is nonblocking");
        let address = listener.local_addr().expect("listener address");
        let requests = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_requests = requests.clone();
        let thread_stop = stop.clone();
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(_) => break,
                };
                stream
                    .set_nonblocking(false)
                    .expect("fixture connection is blocking");
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("fixture connection has a read timeout");
                let mut request_line = String::new();
                BufReader::new(&mut stream)
                    .take(2048)
                    .read_line(&mut request_line)
                    .expect("fixture reads a complete request line");
                let request_line = request_line
                    .split_once("\r\n")
                    .map_or(request_line.as_str(), |(line, _)| line);
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .split('?')
                    .next()
                    .unwrap_or("/")
                    .to_string();
                let count = {
                    let mut requests = thread_requests.lock().expect("request lock");
                    let count = requests.entry(path.clone()).or_default();
                    *count += 1;
                    *count
                };
                let (status, headers, body): (&str, &str, &[u8]) = match (path.as_str(), count) {
                    ("/rate", 1) => ("429 Too Many Requests", "Retry-After: 0\r\n", b""),
                    ("/unstable", 1) | ("/interrupt", 1) => ("500 Internal Server Error", "", b""),
                    ("/expired", _) => ("403 Forbidden", "", b""),
                    ("/init", _) => ("200 OK", "", b"init"),
                    ("/rate", _) => ("200 OK", "", b"rate"),
                    ("/unstable", _) => ("200 OK", "", b"unstable"),
                    ("/interrupt", _) => ("200 OK", "", b"interrupt"),
                    ("/fresh", _) => ("200 OK", "", b"fresh"),
                    _ => ("404 Not Found", "", b""),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        Self {
            base: format!("http://{address}"),
            requests,
            stop,
            thread: Some(thread),
        }
    }

    pub fn count(&self, path: &str) -> usize {
        *self
            .requests
            .lock()
            .expect("request lock")
            .get(path)
            .unwrap_or(&0)
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("fixture server joins");
        }
    }
}

pub fn staging_directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "crunchydl-transfer-{}-{}",
        std::process::id(),
        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

pub fn request(id: &str) -> SegmentRequest {
    SegmentRequest::new(
        format!("https://example.invalid/{id}?secret"),
        None,
        id,
        None,
    )
}

pub fn server_plan(server: &FixtureServer, paths: &[&str]) -> RepresentationTransferPlan {
    RepresentationTransferPlan {
        media_id: "EP1".to_string(),
        version_id: "V1".to_string(),
        plan_fingerprint: "plan".to_string(),
        representation_fingerprint: "0123456789abcdef".to_string(),
        track: None,
        init: SegmentRequest::new(
            format!("{}/init?token=one", server.base),
            None,
            "init",
            Some(4),
        ),
        segments: paths
            .iter()
            .map(|path| {
                SegmentRequest::new(
                    format!("{}{path}?token=one", server.base),
                    None,
                    path.trim_start_matches('/'),
                    None,
                )
            })
            .collect(),
    }
}
