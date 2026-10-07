use super::*;
use std::time::Duration;
async fn transfer(
    asset: &Asset,
    url: &str,
    https_only: bool,
    root: &Path,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(u64),
) -> Result<Installed> {
    let client = reqwest::Client::builder()
        .https_only(https_only)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(1800))
        .build()?;
    let mut response = client.get(url).send().await?.error_for_status()?;
    ensure!(
        response.content_length().is_none_or(|n| n == asset.bytes),
        "Server returned an unexpected model size"
    );
    let mut install = Install::new(root, asset.name)?;
    while let Some(chunk) = response.chunk().await? {
        install.write(&chunk, asset.bytes, cancelled)?;
        progress(install.received);
    }
    install.finish(asset.bytes, asset.checksum(), cancelled)
}
pub(super) fn run(
    asset: &Asset,
    root: &Path,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(u64),
) -> Result<Installed> {
    let url = asset
        .url
        .context("This model is supplied in the app bundle; import a prepared copy")?;
    run_url(asset, url, true, root, cancelled, progress)
}
fn run_url(
    asset: &Asset,
    url: &str,
    https_only: bool,
    root: &Path,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(u64),
) -> Result<Installed> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result=rt.block_on(async {
        tokio::select! {
            biased;
            _ = async {loop {if cancelled(){break;}tokio::time::sleep(Duration::from_millis(50)).await;}} => anyhow::bail!("Cancelled"),
            result=transfer(asset,url,https_only,root,cancelled,progress)=>result,
        }
    });
    rt.shutdown_timeout(Duration::from_millis(100));
    result
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
    fn server(reply: Vec<u8>, stall: bool) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request);
            let _ = socket.write_all(&reply);
            if stall {
                thread::sleep(Duration::from_millis(700));
            }
        });
        (url, handle)
    }
    #[test]
    fn http_failure_truncated_body_and_cancelled_stall_preserve_installed_models() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("models")).unwrap();
        let asset = Id::Detector.asset();
        let dest = dir.path().join("models").join(asset.name);
        std::fs::write(&dest, b"previous").unwrap();
        for response in [
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\nabc",
                asset.bytes
            )
            .into_bytes(),
        ] {
            let (url, handle) = server(response, false);
            assert!(run_url(asset, &url, false, dir.path(), &|| false, &|_| {}).is_err());
            handle.join().unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), b"previous");
            assert_eq!(
                std::fs::read_dir(dir.path().join("models"))
                    .unwrap()
                    .count(),
                1
            );
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
            asset,
            &url,
            false,
            dir.path(),
            &|| cancel.load(Ordering::Acquire),
            &|_| {},
        );
        assert!(result.is_err());
        assert!(
            started.elapsed() < Duration::from_millis(600),
            "Cancellation must interrupt stalled body reads"
        );
        timer.join().unwrap();
        handle.join().unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"previous");
        assert_eq!(
            std::fs::read_dir(dir.path().join("models"))
                .unwrap()
                .count(),
            1
        );
    }
    #[test]
    fn cancellation_can_interrupt_waiting_for_response_headers() {
        let dir = tempfile::tempdir().unwrap();
        let (url, handle) = server(Vec::new(), true);
        let start = Instant::now();
        let result = run_url(
            Id::People.asset(),
            &url,
            false,
            dir.path(),
            &|| start.elapsed() > Duration::from_millis(150),
            &|_| {},
        );
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_millis(600));
        handle.join().unwrap();
        assert!(!dir.path().join("models").exists());
    }
}
