use super::*;
use std::time::Duration;
async fn fetch(url: &str, https_only: bool) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .https_only(https_only)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .user_agent(concat!("PhotoSort-Rust/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10")
        .send()
        .await?;
    let status = response.status();
    ensure!(
        status == reqwest::StatusCode::OK,
        "Release service returned HTTP {status}; no update result is available"
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|n| n <= MAX_RESPONSE as u64),
        "Release response exceeds 2 MiB"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_RESPONSE,
            "Release response exceeds 2 MiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn request(url: &str, https_only: bool, cancelled: &AtomicBool) -> Result<Vec<u8>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = rt.block_on(async {
        tokio::select! {
            biased;
            _ = async { loop { if cancelled.load(Ordering::Acquire) { break; } tokio::time::sleep(Duration::from_millis(25)).await; } } => anyhow::bail!("Cancelled"),
            result = fetch(url, https_only) => result,
        }
    });
    rt.shutdown_timeout(Duration::from_millis(100));
    result
}
pub(super) fn check(source: &Source, current: &str, cancelled: &AtomicBool) -> Result<State> {
    let bytes = request(&source.api_url(), true, cancelled)?;
    ensure!(!cancelled.load(Ordering::Acquire), "Cancelled");
    evaluate(source, current, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        time::Instant,
    };
    fn server(
        response: Vec<u8>,
        stall: bool,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut buffer = [0; 4096];
            let n = socket.read(&mut buffer).unwrap();
            tx.send(String::from_utf8_lossy(&buffer[..n]).into())
                .unwrap();
            let _ = socket.write_all(&response);
            if stall {
                thread::sleep(Duration::from_millis(700));
            }
        });
        (url, rx, handle)
    }
    #[test]
    fn bounded_network_reader_handles_success_http_errors_redirects_and_truncation() {
        let body = br#"{"tag_name":"v0.2.0","draft":false,"prerelease":false}"#;
        let (url, rx, handle) = server(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                String::from_utf8_lossy(body)
            )
            .into_bytes(),
            false,
        );
        let bytes = request(&url, false, &AtomicBool::new(false)).unwrap();
        assert!(matches!(
            evaluate(&Source::new("example/rust").unwrap(), "0.1.0", &bytes).unwrap(),
            State::Available(_)
        ));
        let headers = rx.recv().unwrap().to_lowercase();
        assert!(headers.contains("user-agent: photosort-rust/"));
        assert!(headers.contains("accept: application/vnd.github+json"));
        assert!(!headers.contains("authorization:"));
        handle.join().unwrap();
        for response in [
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_string(),
            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n".into(),
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/\r\nContent-Length: 0\r\n\r\n"
                .into(),
            "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nabc".into(),
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                MAX_RESPONSE + 1
            ),
            format!(
                "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{}",
                "x".repeat(MAX_RESPONSE + 1)
            ),
        ] {
            let (url, rx, handle) = server(response.into_bytes(), false);
            assert!(request(&url, false, &AtomicBool::new(false)).is_err());
            let _ = rx.recv().unwrap();
            handle.join().unwrap();
        }
    }
    #[test]
    fn shared_worker_shutdown_cancels_stalled_headers_and_body_without_late_result() {
        for response in [
            vec![],
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\na".to_vec(),
        ] {
            let (url, received, handle) = server(response, true);
            let mut worker = Worker::new(Arc::new(|| {}));
            worker
                .start_task(move |cancelled| {
                    request(&url, false, cancelled)?;
                    Ok(State::Current {
                        latest: "1.0.0".into(),
                    })
                })
                .unwrap();
            received.recv_timeout(Duration::from_secs(2)).unwrap();
            let start = Instant::now();
            worker.shutdown();
            assert!(start.elapsed() < Duration::from_millis(600));
            assert_eq!(worker.state(), State::Cancelled);
            handle.join().unwrap();
        }
    }
}
