//! Application-wide release checks. No photo data, model runtime or file mutation.
use anyhow::{Context, Result, ensure};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(feature = "updates")]
mod network;

const INTERVAL: u64 = 24 * 60 * 60;
const MAX_RESPONSE: usize = 2 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub automatic: bool,
    pub last_attempt: u64,
    pub repository: Option<String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            automatic: true,
            last_attempt: 0,
            repository: None,
        }
    }
}
impl Preferences {
    pub fn due(&self, source: &Source, current: &str, now: u64) -> bool {
        self.automatic
            && Version::parse(current).is_ok_and(|v| v.pre.is_empty())
            && (self.repository.as_deref() != Some(source.repository.as_str())
                || self.last_attempt == 0
                || now < self.last_attempt
                || now.saturating_sub(self.last_attempt) >= INTERVAL)
    }
    pub fn attempted(&mut self, source: &Source, now: u64) {
        self.repository = Some(source.repository.clone());
        self.last_attempt = now;
    }
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Clone, Debug)]
pub struct Source {
    repository: String,
}
impl Source {
    /// Explicit build identity, never inferred from a Git remote.
    pub fn new(repository: &str) -> Result<Self> {
        let parts: Vec<_> = repository.split('/').collect();
        ensure!(
            parts.len() == 2
                && parts.iter().all(|p| !p.is_empty()
                    && p.len() <= 100
                    && *p != "."
                    && *p != ".."
                    && p.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))),
            "Expected a GitHub owner/repository"
        );
        Ok(Self {
            repository: repository.into(),
        })
    }
    pub fn configured() -> std::result::Result<Self, String> {
        if !cfg!(feature = "updates") {
            return Err("Online update checks are not included in this build.".into());
        }
        let repo = option_env!("PHOTOSORT_RELEASE_REPOSITORY").ok_or_else(|| {
            "This development build has no Rust release source configured.".to_string()
        })?;
        Self::new(repo).map_err(|e| format!("Invalid release configuration: {e}"))
    }
    pub fn repository(&self) -> &str {
        &self.repository
    }
    pub fn api_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/releases/latest",
            self.repository
        )
    }
    fn release_url(&self, version: &str) -> String {
        format!(
            "https://github.com/{}/releases/tag/{version}",
            self.repository
        )
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub notes: String,
    pub notes_truncated: bool,
    pub published_at: String,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub enum State {
    #[default]
    Idle,
    Checking,
    Cancelled,
    Available(Arc<Release>),
    Current {
        latest: String,
    },
    Failed(String),
    Unavailable(String),
}
#[derive(Deserialize)]
struct Response {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    body: Option<String>,
    published_at: Option<String>,
}
pub fn evaluate(source: &Source, current: &str, bytes: &[u8]) -> Result<State> {
    ensure!(
        bytes.len() <= MAX_RESPONSE,
        "Release response exceeds 2 MiB"
    );
    let data: Response = serde_json::from_slice(bytes).context("Invalid release response")?;
    ensure!(
        !data.draft && !data.prerelease,
        "The release source did not return a stable release"
    );
    ensure!(data.tag_name.len() <= 128, "Release tag is too long");
    let version = Version::parse(data.tag_name.strip_prefix('v').unwrap_or(&data.tag_name))
        .context("Release tag must use semantic versioning")?;
    ensure!(version.pre.is_empty(), "The release tag is a prerelease");
    let current = Version::parse(current).context("Invalid application version")?;
    // Build metadata does not change SemVer precedence.
    if version.cmp_precedence(&current).is_gt() {
        // Bound layout work before publishing to the UI. The release page retains
        // the full notes; slicing happens once on the update worker, not per frame.
        let mut notes = data.body.unwrap_or_default();
        let notes_truncated = notes.len() > 16 * 1024;
        if notes_truncated {
            let mut end = 16 * 1024;
            while !notes.is_char_boundary(end) {
                end -= 1;
            }
            notes.truncate(end);
        }
        let published_at = data
            .published_at
            .and_then(|date| chrono::DateTime::parse_from_rfc3339(&date).ok())
            .map(|date| date.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        Ok(State::Available(Arc::new(Release {
            version: version.to_string(),
            url: source.release_url(&data.tag_name),
            notes,
            notes_truncated,
            published_at,
        })))
    } else {
        Ok(State::Current {
            latest: version.to_string(),
        })
    }
}

pub struct Worker {
    state: Arc<Mutex<State>>,
    cancelled: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    wake: Arc<dyn Fn() + Send + Sync>,
    stopped: bool,
}
impl Worker {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            state: Arc::default(),
            cancelled: Arc::default(),
            thread: None,
            wake,
            stopped: false,
        }
    }
    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }
    pub fn running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
    pub fn unavailable(&mut self, reason: String) {
        if !self.running() {
            *self.state.lock().unwrap() = State::Unavailable(reason);
        }
    }
    pub fn start(&mut self, source: Source, current: String) -> Result<bool> {
        self.start_task(move |cancelled| {
            #[cfg(feature = "updates")]
            {
                network::check(&source, &current, cancelled)
            }
            #[cfg(not(feature = "updates"))]
            {
                let _ = (source, current, cancelled);
                anyhow::bail!("Online update checks are not included in this build")
            }
        })
    }
    fn start_task(
        &mut self,
        check: impl FnOnce(&AtomicBool) -> Result<State> + Send + 'static,
    ) -> Result<bool> {
        ensure!(!self.stopped, "Update worker has stopped");
        if self.running() {
            return Ok(false);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.cancelled = Arc::new(AtomicBool::new(false));
        let (state, cancelled, wake) = (
            self.state.clone(),
            self.cancelled.clone(),
            self.wake.clone(),
        );
        *state.lock().unwrap() = State::Checking;
        let result = thread::Builder::new()
            .name("photosort-updates".into())
            .spawn(move || {
                let result = check(&cancelled);
                let mut state = state.lock().unwrap();
                // Cancel and publication share this lock, including cancellation after HTTP completes.
                if !cancelled.load(Ordering::Acquire) {
                    *state = result.unwrap_or_else(|e| State::Failed(format!("{e:#}")));
                }
                drop(state);
                wake();
            });
        match result {
            Ok(thread) => {
                self.thread = Some(thread);
                Ok(true)
            }
            Err(error) => {
                *self.state.lock().unwrap() = State::Failed(error.to_string());
                Err(error.into())
            }
        }
    }
    pub fn cancel(&self) {
        let mut state = self.state.lock().unwrap();
        self.cancelled.store(true, Ordering::Release);
        if matches!(*state, State::Checking) {
            *state = State::Cancelled;
        }
    }
    pub fn shutdown(&mut self) {
        self.cancel();
        self.stopped = true;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::worker_patience;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    fn payload(tag: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"tag_name":tag,"draft":false,"prerelease":false,"body":"<img src='https://invalid.example/a'>","html_url":"javascript:wrong"})).unwrap()
    }
    #[test]
    fn release_previews_bound_layout_work_without_breaking_unicode() {
        let source = Source::new("example/rust").unwrap();
        for notes in [
            "🏞".repeat(100_000),
            "a".repeat(16_383) + "🏞",
            "short notes".into(),
        ] {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "tag_name":"v1.0.0", "draft":false,"prerelease":false,
                "body":notes, "published_at":"2026-10-02T10:30:00Z"
            }))
            .unwrap();
            let State::Available(release) = evaluate(&source, "0.1.0", &bytes).unwrap() else {
                panic!()
            };
            assert!(release.notes.len() <= 16 * 1024);
            assert!(notes.starts_with(&release.notes));
            assert_eq!(release.notes_truncated, notes.len() > 16 * 1024);
            assert_eq!(release.published_at, "2026-10-02");
        }
        let bytes = serde_json::to_vec(&serde_json::json!({
            "tag_name":format!("1.0.0+{}", "a".repeat(200)), "draft":false,"prerelease":false
        }))
        .unwrap();
        assert!(evaluate(&source, "0.1.0", &bytes).is_err());
    }
    #[test]
    fn semantic_versions_do_not_offer_downgrades_or_build_metadata_updates() {
        let source = Source::new("example/PhotoSort").unwrap();
        for (tag, current, available) in [
            ("v0.2.0", "0.1.0", true),
            ("v1.0.0", "1.0.0-rc.2", true),
            ("1.0.0", "1.0.0", false),
            ("1.0.0", "2.0.0", false),
            ("1.0.0+new", "1.0.0+old", false),
            ("1.10.0", "1.9.0", true),
        ] {
            let state = evaluate(&source, current, &payload(tag)).unwrap();
            assert_eq!(matches!(state, State::Available(_)), available);
            if let State::Available(release) = state {
                assert_eq!(
                    release.url,
                    format!("https://github.com/example/PhotoSort/releases/tag/{tag}")
                );
                assert!(release.notes.starts_with("<img")); // plain text, never parsed as HTML
            }
        }
        for tag in ["dev", "1.2", "v1.2.3-rc.1", "1.0.0/../../bad", "vv1.0.0"] {
            assert!(evaluate(&source, "0.1.0", &payload(tag)).is_err());
        }
        assert!(evaluate(&source, "dev", &payload("1.0.0")).is_err());
        for body in [
            br#"{"tag_name":"1.0.0","draft":true,"prerelease":false}"#.as_slice(),
            br#"{"tag_name":"1.0.0","draft":false,"prerelease":true}"#,
            b"{}",
            b"null",
            b"bad",
        ] {
            assert!(evaluate(&source, "0.1.0", body).is_err());
        }
        assert!(evaluate(&source, "0.1.0", &vec![b' '; MAX_RESPONSE + 1]).is_err());
    }
    #[test]
    fn release_identity_and_daily_schedule_handle_channel_changes_and_clock_rollback() {
        for repo in [
            "",
            "/repo",
            "owner/",
            "a/b/c",
            "https://github.com/a/b",
            "a/..",
            "a/b?x",
            "a/b#x",
            "a/b%2fc",
        ] {
            assert!(Source::new(repo).is_err(), "{repo}");
        }
        let source = Source::new("owner/repo").unwrap();
        let mut preferences = Preferences::default();
        assert!(preferences.due(&source, "1.0.0", 100_000));
        assert!(!preferences.due(&source, "1.0.0-beta.1", 100_000));
        preferences.attempted(&source, 100_000);
        assert!(!preferences.due(&source, "1.0.0", 100_001));
        assert!(preferences.due(&source, "1.0.0", 100_000 + INTERVAL));
        assert!(preferences.due(&source, "1.0.0", 99_000));
        assert!(preferences.due(&Source::new("owner/other").unwrap(), "1.0.0", 100_001));
        preferences.automatic = false;
        assert!(!preferences.due(&source, "1.0.0", 1_000_000));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let mut saved = crate::core::preferences::Preferences {
            updates: preferences,
            ..Default::default()
        };
        saved.save(&path).unwrap();
        assert_eq!(
            crate::core::preferences::Preferences::load(&path).unwrap(),
            saved
        );
        std::fs::write(&path, b"[1,{}]").unwrap();
        saved = crate::core::preferences::Preferences::load(&path).unwrap();
        assert_eq!(saved.updates, Preferences::default());
    }
    #[test]
    fn one_worker_coalesces_all_callers_and_discards_cancelled_late_completion() {
        let mut worker = Worker::new(Arc::new(|| {}));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        assert!(
            worker
                .start_task(move |_| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(State::Current {
                        latest: "1.0.0".into(),
                    })
                })
                .unwrap()
        );
        started_rx.recv_timeout(worker_patience()).unwrap();
        for _ in crate::core::model::Step::ALL {
            assert!(!worker.start_task(|_| panic!("duplicate request")).unwrap());
        }
        worker.cancel();
        assert_eq!(worker.state(), State::Cancelled);
        release_tx.send(()).unwrap();
        let start = Instant::now();
        while worker.running() {
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(worker.state(), State::Cancelled);
        assert!(
            worker
                .start_task(|_| Ok(State::Current {
                    latest: "2.0.0".into()
                }))
                .unwrap()
        );
        while worker.running() {
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            worker.state(),
            State::Current {
                latest: "2.0.0".into()
            }
        );
        worker.shutdown();
        assert!(worker.start_task(|_| panic!("stopped")).is_err());
    }
}
