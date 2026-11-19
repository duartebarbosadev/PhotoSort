use super::*;
use std::time::Duration;
/// What a download must turn out to be, and the name it is installed under.
struct Pinned<'a> {
    name: &'a str,
    bytes: u64,
    sha256: &'a str,
}
/// Why a failed transfer leaves nothing to continue from.
const UNUSABLE: &str = "The partial download could not be continued and was discarded";
/// Streams the file into `install`, continuing after the bytes it already
/// holds when the server agrees to (HTTP Range). Returns whether what was
/// received may be kept for a later attempt when this one fails.
async fn transfer(
    pinned: &Pinned<'_>,
    url: &str,
    https_only: bool,
    install: &mut Install,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(u64),
) -> (bool, Result<()>) {
    let mut resumable = true;
    let result = async {
        // All of it was received before the last attempt stopped: only the
        // check is left.
        if install.received == pinned.bytes {
            return Ok(());
        }
        let client = reqwest::Client::builder()
            .https_only(https_only)
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            // At least 50 KB/s on average, so a slow but working connection can
            // still finish the 535 MB aesthetic model.
            .timeout(Duration::from_secs(1800).max(Duration::from_secs(pinned.bytes / 50_000)))
            .build()?;
        let mut request = client.get(url);
        if install.received > 0 {
            request = request.header(
                reqwest::header::RANGE,
                format!("bytes={}-", install.received),
            );
        }
        let response = request.send().await?;
        if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            resumable = false;
            anyhow::bail!(UNUSABLE);
        }
        let mut response = checked(response)?;
        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            // The rest of this very file, from where the partial one ends.
            let range = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default();
            if install.received == 0
                || !range.starts_with(&format!("bytes {}-", install.received))
                || !range.ends_with(&format!("/{}", pinned.bytes))
            {
                resumable = false;
                anyhow::bail!(UNUSABLE);
            }
        } else if install.received > 0 {
            // The server sent the whole file instead: start over with it.
            install.restart()?;
        }
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n == pinned.bytes - install.received),
            "Server returned an unexpected model size"
        );
        progress(install.received);
        while let Some(chunk) = response.chunk().await? {
            if install.received + chunk.len() as u64 > pinned.bytes {
                resumable = false;
            }
            install.write(&chunk, pinned.bytes, cancelled)?;
            progress(install.received);
        }
        Ok(())
    }
    .await;
    (resumable, result)
}
pub(super) fn run(
    asset: &Asset,
    root: &Path,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(u64),
) -> Result<Installed> {
    let pinned = Pinned {
        name: asset.name,
        bytes: asset.bytes,
        sha256: asset.checksum(),
    };
    run_url(&pinned, asset.url, true, root, cancelled, progress)
}
/// The response, or why the server refused it, in words: a file the server no
/// longer has says so, instead of only an HTTP status.
fn checked(response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if matches!(
        status,
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
    ) {
        let host = response.url().host_str().unwrap_or("The server").to_owned();
        anyhow::bail!(
            "{host} no longer offers this file (HTTP {}). It may be back later, or a newer PhotoSort may download it from elsewhere",
            status.as_u16()
        );
    }
    Ok(response.error_for_status()?)
}
/// Downloads one pinned file to `destination`, which must not exist yet, and
/// keeps it only when its size and SHA-256 match. Runtime packages use this.
pub(super) fn fetch(
    url: &str,
    bytes: u64,
    sha256: &str,
    destination: &Path,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(u64),
) -> Result<()> {
    async fn transfer(
        url: &str,
        bytes: u64,
        sha256: &str,
        destination: &Path,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(u64),
    ) -> Result<()> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(1800).max(Duration::from_secs(bytes / 50_000)))
            .build()?;
        let mut response = checked(client.get(url).send().await?)?;
        ensure!(
            response.content_length().is_none_or(|n| n == bytes),
            "Server returned an unexpected download size"
        );
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .with_context(|| format!("Create {}", crate::core::text::shown(destination)))?;
        let mut hash = Sha256::new();
        let mut received = 0u64;
        while let Some(chunk) = response.chunk().await? {
            ensure!(!cancelled(), "Cancelled");
            ensure!(
                received + chunk.len() as u64 <= bytes,
                "Download is larger than expected"
            );
            file.write_all(&chunk)?;
            hash.update(&chunk);
            received += chunk.len() as u64;
            progress(received);
        }
        ensure!(
            received == bytes,
            "Download is incomplete: {received} of {bytes} bytes"
        );
        ensure!(
            hex(&hash.finalize()) == sha256,
            "Download SHA-256 verification failed"
        );
        file.sync_all()?;
        Ok(())
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = rt.block_on(async {
        tokio::select! {
            biased;
            _ = async { loop { if cancelled() { break; } tokio::time::sleep(Duration::from_millis(50)).await; } } => anyhow::bail!("Cancelled"),
            result = transfer(url, bytes, sha256, destination, cancelled, progress) => result,
        }
    });
    rt.shutdown_timeout(Duration::from_millis(100));
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}
/// An interrupted download (cancelled, or the connection lost) keeps what it
/// received beside the models, and the next one continues from there. The
/// file is installed only once all of it passed the size and SHA-256 check; a
/// partial file that fails it is removed, so the download after starts clean.
fn run_url(
    pinned: &Pinned,
    url: &str,
    https_only: bool,
    root: &Path,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(u64),
) -> Result<Installed> {
    let mut install = Install::resume(root, pinned.name, pinned.bytes, cancelled)?;
    // A download that continues starts from what it already has.
    progress(install.received);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (resumable, result) = rt.block_on(async {
        tokio::select! {
            biased;
            _ = async { loop { if cancelled() { break; } tokio::time::sleep(Duration::from_millis(50)).await; } } => (true, Err(anyhow::anyhow!("Cancelled"))),
            result = transfer(pinned, url, https_only, &mut install, cancelled, progress) => result,
        }
    });
    rt.shutdown_timeout(Duration::from_millis(100));
    match result {
        Ok(()) if cancelled() => {
            install.keep();
            anyhow::bail!("Cancelled")
        }
        Ok(()) if install.received == pinned.bytes => {
            install.finish(pinned.bytes, pinned.sha256, cancelled)
        }
        // The body ended early without an error: the rest can still follow.
        Ok(()) => {
            let received = install.received;
            install.keep();
            anyhow::bail!("Model is incomplete: {received} of {} bytes", pinned.bytes)
        }
        Err(error) => {
            if resumable {
                install.keep();
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::Instant,
    };
    /// Answers one request with `reply` and returns what was asked.
    fn server(reply: Vec<u8>, stall: bool) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = [0; 4096];
            let read = socket.read(&mut request).unwrap_or(0);
            let _ = socket.write_all(&reply);
            // Stall well past the cancellation bound below, so a slow shared
            // CI machine cannot mistake the server finishing for cancellation.
            if stall {
                thread::sleep(Duration::from_secs(3));
            }
            String::from_utf8_lossy(&request[..read]).to_ascii_lowercase()
        });
        (url, handle)
    }
    fn pinned(asset: &Asset) -> Pinned<'_> {
        Pinned {
            name: asset.name,
            bytes: asset.bytes,
            sha256: asset.checksum(),
        }
    }
    /// The names in the models folder.
    fn files(root: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(root.join("models"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
    #[test]
    fn http_failure_truncated_body_and_cancelled_stall_preserve_installed_models() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("models")).unwrap();
        let asset = Id::Detector.asset();
        let dest = dir.path().join("models").join(asset.name);
        let partial = partial_path(dir.path(), asset.name);
        std::fs::write(&dest, b"previous").unwrap();
        for (response, kept) in [
            (
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
                None,
            ),
            // What a lost connection delivered is kept for the next attempt.
            (
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\nabc",
                    asset.bytes
                )
                .into_bytes(),
                Some(b"abc"),
            ),
        ] {
            let (url, handle) = server(response, false);
            assert!(run_url(&pinned(asset), &url, false, dir.path(), &|| false, &|_| {}).is_err());
            handle.join().unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), b"previous");
            assert_eq!(
                std::fs::read(&partial).ok().as_deref(),
                kept.map(|k| &k[..])
            );
            assert_eq!(files(dir.path()).len(), 1 + usize::from(kept.is_some()));
        }
        let (url, handle) = server(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\na",
                asset.bytes
            )
            .into_bytes(),
            true,
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        let timer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            token.store(true, Ordering::Release);
        });
        let started = Instant::now();
        let result = run_url(
            &pinned(asset),
            &url,
            false,
            dir.path(),
            &|| cancel.load(Ordering::Acquire),
            &|_| {},
        );
        assert!(result.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "Cancellation must interrupt stalled body reads"
        );
        timer.join().unwrap();
        // It asked for the rest, and the server sent the file from its start
        // instead: the partial file starts over with what arrived.
        assert!(handle.join().unwrap().contains("range: bytes=3-"));
        assert_eq!(std::fs::read(&dest).unwrap(), b"previous");
        assert_eq!(std::fs::read(&partial).unwrap(), b"a");
        assert_eq!(files(dir.path()).len(), 2);
    }
    #[test]
    fn an_interrupted_download_continues_where_it_stopped_and_is_verified_whole() {
        const ABCDEF: &str = "bef57ec7f53a6d40beb640a780a639c83bc29ac8a9816f1fc6c5c6dcd93c4721";
        let dir = tempfile::tempdir().unwrap();
        let model = Pinned {
            name: "a.onnx",
            bytes: 6,
            sha256: ABCDEF,
        };
        let dest = dir.path().join("models/a.onnx");
        let partial = partial_path(dir.path(), "a.onnx");
        let rest = b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 3-5/6\r\nContent-Length: 3\r\nConnection: close\r\n\r\ndef";
        let download = |reply: &[u8]| {
            let (url, handle) = server(reply.to_vec(), false);
            let result = run_url(&model, &url, false, dir.path(), &|| false, &|_| {});
            (result, handle.join().unwrap())
        };
        // The connection is lost after three of the six bytes.
        let (result, _) =
            download(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabc");
        assert!(result.is_err());
        assert_eq!(std::fs::read(&partial).unwrap(), b"abc");
        assert!(!dest.exists(), "Half a model is never installed");
        // The next download asks only for the rest and installs the whole.
        let (result, request) = download(rest);
        assert!(request.contains("range: bytes=3-"), "{request}");
        assert_eq!(result.unwrap().path, dest);
        assert_eq!(std::fs::read(&dest).unwrap(), b"abcdef");
        assert_eq!(files(dir.path()), ["a.onnx"]);
        // A partial file with other content fails the check of the whole
        // file: nothing is installed and it is removed, so the download
        // after it starts clean.
        std::fs::remove_file(&dest).unwrap();
        std::fs::write(&partial, b"xyz").unwrap();
        let (result, _) = download(rest);
        assert!(result.is_err());
        assert!(files(dir.path()).is_empty());
        // So is one the server will not continue.
        std::fs::write(&partial, b"abc").unwrap();
        let (result, _) = download(
            b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-2/9\r\nContent-Length: 3\r\nConnection: close\r\n\r\ndef",
        );
        assert!(result.is_err());
        assert!(files(dir.path()).is_empty());
    }
    #[test]
    fn cancellation_can_interrupt_waiting_for_response_headers() {
        let dir = tempfile::tempdir().unwrap();
        let (url, handle) = server(Vec::new(), true);
        let start = Instant::now();
        let result = run_url(
            &pinned(Id::People.asset()),
            &url,
            false,
            dir.path(),
            &|| start.elapsed() > Duration::from_millis(150),
            &|_| {},
        );
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
        handle.join().unwrap();
        assert!(files(dir.path()).is_empty());
    }
}
