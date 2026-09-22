//! Where content comes from and how it reaches the database.
//!
//! Git is the only durable store: the database is a materialized view of
//! one content revision and can always be rebuilt from it. In production the
//! server pulls the content repository as a tarball, pinned to the commit a
//! branch pointed at; in development it reads a directory.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use hldr_core::api::GitHubQuota;
use hldr_core::events::NewEvent;
use hldr_core::github::RateLimit;
use hldr_core::{Db, SyncReport, store};

pub const GITHUB_API: &str = "https://api.github.com";
const DEFAULT_REF: &str = "main";
const DEFAULT_POLL: Duration = Duration::from_secs(300);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Compressed archive, as downloaded.
const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
/// Sum of the file sizes the archive declares once unpacked.
const MAX_CONTENT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentSource {
    Dir(PathBuf),
    GitHub {
        api: String,
        repo: String,
        git_ref: String,
    },
}

impl ContentSource {
    /// `HLDR_CONTENT_REPO` (with `HLDR_CONTENT_REF`) selects a GitHub
    /// repository; `HLDR_CONTENT_DIR` a local directory. One is required.
    pub fn from_env() -> Result<Self, String> {
        let var = |name| std::env::var(name).ok().filter(|v: &String| !v.is_empty());
        match (var("HLDR_CONTENT_REPO"), var("HLDR_CONTENT_DIR")) {
            (Some(_), Some(_)) => Err("set HLDR_CONTENT_REPO or HLDR_CONTENT_DIR, not both".into()),
            (Some(repo), None) => Self::github(
                GITHUB_API,
                &repo,
                &var("HLDR_CONTENT_REF").unwrap_or_else(|| DEFAULT_REF.to_owned()),
            ),
            (None, Some(dir)) => Ok(Self::Dir(PathBuf::from(dir))),
            (None, None) => Err("set HLDR_CONTENT_REPO (owner/name) or HLDR_CONTENT_DIR".into()),
        }
    }

    /// Both values end up in a URL path, so they are checked, not escaped.
    pub fn github(api: &str, repo: &str, git_ref: &str) -> Result<Self, String> {
        let safe = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
        let valid_repo = repo.split_once('/').is_some_and(|(owner, name)| {
            [owner, name]
                .iter()
                .all(|part| !part.is_empty() && !part.starts_with('.') && part.chars().all(safe))
        });
        if !valid_repo {
            return Err(format!(
                "HLDR_CONTENT_REPO must be owner/name, got {repo:?}"
            ));
        }
        let valid_ref = !git_ref.is_empty()
            && !git_ref.starts_with('/')
            && !git_ref.ends_with('/')
            && !git_ref.contains("..")
            && git_ref.chars().all(|c| safe(c) || c == '/');
        if !valid_ref {
            return Err(format!(
                "HLDR_CONTENT_REF is not a branch or tag name: {git_ref:?}"
            ));
        }
        Ok(Self::GitHub {
            api: api.trim_end_matches('/').to_owned(),
            repo: repo.to_owned(),
            git_ref: git_ref.to_owned(),
        })
    }

    /// The GitHub repository and ref, when content comes from one.
    pub fn origin(&self) -> Option<(String, String)> {
        match self {
            Self::Dir(_) => None,
            Self::GitHub { repo, git_ref, .. } => Some((repo.clone(), git_ref.clone())),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Dir(path) => format!("dir:{}", path.display()),
            Self::GitHub { repo, git_ref, .. } => format!("github.com/{repo}@{git_ref}"),
        }
    }
}

/// `HLDR_CONTENT_POLL`, such as `300s`, `5m` or `1h`; `0` turns polling off.
pub fn poll_interval() -> Result<Option<Duration>, String> {
    match std::env::var("HLDR_CONTENT_POLL") {
        Ok(value) => parse_interval(&value),
        Err(_) => Ok(Some(DEFAULT_POLL)),
    }
}

fn parse_interval(value: &str) -> Result<Option<Duration>, String> {
    let invalid = || format!("HLDR_CONTENT_POLL must look like 30s, 5m or 1h, got {value:?}");
    if value == "0" {
        return Ok(None);
    }
    let split = value
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(invalid)?;
    let (amount, unit) = value.split_at(split);
    let amount: u64 = amount.parse().map_err(|_| invalid())?;
    let seconds = match unit {
        "s" => amount,
        "m" => amount.checked_mul(60).ok_or_else(invalid)?,
        "h" => amount.checked_mul(3600).ok_or_else(invalid)?,
        _ => return Err(invalid()),
    };
    if seconds == 0 {
        return Ok(None);
    }
    Ok(Some(Duration::from_secs(seconds)))
}

#[derive(Debug)]
pub enum SyncError {
    /// The source could not be read: network, GitHub, or the archive.
    Fetch(String),
    /// GitHub refused for a spent rate limit; the message says when it
    /// resets.
    RateLimited(String),
    /// The content did not index; the previous revision stays served.
    Index(hldr_core::Error),
    /// The branch did not reach the revision the caller expected in time.
    Stale(String),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fetch(message) | Self::RateLimited(message) => write!(f, "fetch: {message}"),
            Self::Index(error) => write!(f, "index: {error}"),
            Self::Stale(message) => write!(f, "stale: {message}"),
        }
    }
}

/// Runs syncs one at a time: a request that arrives during a sync waits for
/// it, then finds the content current.
pub struct Syncer {
    source: ContentSource,
    /// Optional read-only token: authenticated reads are never served from
    /// GitHub's cache and get 5000 requests an hour instead of 60.
    token: Option<String>,
    db: Db,
    running: tokio::sync::Mutex<()>,
    /// How often and how long to wait for an expected revision.
    patience: (Duration, Duration),
    /// The rate limit as GitHub's last answer reported it.
    quota: Arc<Mutex<Option<GitHubQuota>>>,
}

impl Syncer {
    pub fn new(source: ContentSource, db: Db) -> Self {
        Self {
            source,
            token: None,
            db,
            running: tokio::sync::Mutex::new(()),
            patience: (Duration::from_secs(3), Duration::from_secs(60)),
            quota: Arc::default(),
        }
    }

    /// The GitHub rate limit as last reported; `None` before the first
    /// request, and for content read from a directory.
    pub fn quota(&self) -> Option<GitHubQuota> {
        self.quota
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token.filter(|token| !token.trim().is_empty());
        self
    }

    #[cfg(test)]
    fn with_patience(mut self, every: Duration, within: Duration) -> Self {
        self.patience = (every, within);
        self
    }

    pub fn source(&self) -> &ContentSource {
        &self.source
    }

    /// Brings the database to the source's current revision. Returns `None`
    /// when it already was; `force` indexes anyway, as on boot, where the
    /// new binary may materialize content differently.
    pub async fn sync(&self, force: bool) -> Result<Option<SyncReport>, SyncError> {
        self.sync_to(force, None).await
    }

    /// Like [`Syncer::sync`], but first waits for the branch to point at
    /// `expected`, the commit a client just pushed.
    pub async fn sync_to(
        &self,
        force: bool,
        expected: Option<&str>,
    ) -> Result<Option<SyncReport>, SyncError> {
        if let Some(sha) = expected
            && !(sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()))
        {
            return Err(SyncError::Stale(format!("not a commit id: {sha:?}")));
        }
        let _running = self.running.lock().await;
        let served = self
            .db
            .sync_state()
            .await
            .map_err(SyncError::Index)?
            .revision;
        let outcome = self.run(force, expected).await;
        let recorded = match &outcome {
            Ok(Some((_, revision))) => self.db.record_sync(revision.as_deref()).await,
            Ok(None) => self.db.record_sync_attempt(None).await,
            Err(error) => self.db.record_sync_attempt(Some(&error.to_string())).await,
        };
        recorded.map_err(SyncError::Index)?;
        if let Some(event) = sync_event(served.as_deref(), &outcome) {
            record(&self.db, &event).await;
        }
        outcome.map(|done| done.map(|(report, _)| report))
    }

    async fn run(
        &self,
        force: bool,
        expected: Option<&str>,
    ) -> Result<Option<(SyncReport, Option<String>)>, SyncError> {
        match &self.source {
            ContentSource::Dir(dir) => {
                let report = hldr_core::index::sync(self.db.pool(), dir)
                    .await
                    .map_err(SyncError::Index)?;
                Ok(Some((report, None)))
            }
            ContentSource::GitHub { api, repo, git_ref } => {
                let github = GitHub::new(api, repo, self.token.clone(), Arc::clone(&self.quota));
                let resolve = || {
                    let github = github.clone();
                    let git_ref = git_ref.clone();
                    blocking(move || github.resolve(&git_ref))
                };
                let mut sha = resolve().await?;
                if let Some(expected) = expected.map(str::to_ascii_lowercase) {
                    let (every, within) = self.patience;
                    let deadline = tokio::time::Instant::now() + within;
                    while sha != expected {
                        if tokio::time::Instant::now() + every > deadline {
                            return Err(SyncError::Stale(format!(
                                "{git_ref} is at {sha}, not {expected}; \
                                 the branch may have moved on, or GitHub is slow to show the push"
                            )));
                        }
                        tokio::time::sleep(every).await;
                        sha = resolve().await?;
                    }
                }

                let state = self.db.sync_state().await.map_err(SyncError::Index)?;
                if !force && state.synced_at.is_some() && state.revision.as_deref() == Some(&sha) {
                    return Ok(None);
                }

                let dir = tempfile::tempdir().map_err(|err| SyncError::Fetch(err.to_string()))?;
                blocking({
                    let sha = sha.clone();
                    let dest = dir.path().to_owned();
                    move || github.download(&sha, &dest)
                })
                .await?;
                let report = hldr_core::index::sync(self.db.pool(), dir.path())
                    .await
                    .map_err(SyncError::Index)?;
                Ok(Some((report, Some(sha))))
            }
        }
    }
}

/// Writes an event; one that cannot be written is logged and dropped, since
/// events only report what happened.
pub async fn record(db: &Db, event: &NewEvent) {
    if let Err(error) = db.record_event(event).await {
        tracing::warn!(%error, reason = event.reason, "event not recorded");
    }
}

/// What a sync is worth an event for: a change of what is served, or a
/// failure. A sync that changed nothing is not.
fn sync_event(
    served: Option<&str>,
    outcome: &Result<Option<(SyncReport, Option<String>)>, SyncError>,
) -> Option<NewEvent> {
    match outcome {
        Ok(None) => None,
        Ok(Some((report, revision))) => {
            let changes = changes(report);
            let event = match (served, revision.as_deref()) {
                (_, None) if changes.is_empty() => return None,
                (_, None) => NewEvent::normal("Synced", format!("synced the directory: {changes}")),
                (Some(old), Some(new)) if old == new && changes.is_empty() => return None,
                (Some(old), Some(new)) if old == new => {
                    NewEvent::normal("Synced", format!("reindexed {}: {changes}", short(new)))
                }
                (old, Some(new)) => NewEvent::normal(
                    "Synced",
                    format!(
                        "{} → {}: {}",
                        old.map_or("nothing", short),
                        short(new),
                        if changes.is_empty() {
                            "no resource changed"
                        } else {
                            &changes
                        }
                    ),
                ),
            };
            Some(event.at(revision.as_deref()))
        }
        Err(error @ SyncError::RateLimited(_)) => {
            Some(NewEvent::warning("RateLimited", error.to_string()).at(served))
        }
        Err(error) => Some(NewEvent::warning("SyncFailed", error.to_string()).at(served)),
    }
}

/// Such as `Page 1 upserted, Theme 2 deleted`: the kinds a sync changed.
fn changes(report: &SyncReport) -> String {
    let mut parts = Vec::new();
    for (kind, counts) in &report.kinds {
        if counts.upserted > 0 {
            parts.push(format!("{kind} {} upserted", counts.upserted));
        }
        if counts.deleted > 0 {
            parts.push(format!("{kind} {} deleted", counts.deleted));
        }
    }
    parts.join(", ")
}

/// A commit as the CLI prints one: its first 12 digits.
pub fn short(revision: &str) -> &str {
    revision.get(..12).unwrap_or(revision)
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, SyncError> + Send + 'static,
) -> Result<T, SyncError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| SyncError::Fetch(err.to_string()))?
}

#[derive(Clone)]
struct GitHub {
    agent: ureq::Agent,
    base: String,
    token: Option<String>,
    quota: Arc<Mutex<Option<GitHubQuota>>>,
}

impl GitHub {
    fn new(
        api: &str,
        repo: &str,
        token: Option<String>,
        quota: Arc<Mutex<Option<GitHubQuota>>>,
    ) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .http_status_as_error(false)
            .user_agent(format!("hldr-server/{}", hldr_core::VERSION))
            .build()
            .into();
        Self {
            agent,
            base: format!("{api}/repos/{repo}"),
            token,
            quota,
        }
    }

    /// The body of a successful answer. Every answer that reports the rate
    /// limit updates the quota; a spent one says when it resets, so
    /// `last_error` tells how long the site stays behind.
    fn call(
        &self,
        request: ureq::RequestBuilder<ureq::typestate::WithoutBody>,
        what: &str,
    ) -> Result<ureq::Body, SyncError> {
        let response = request
            .call()
            .map_err(|err| SyncError::Fetch(format!("{what}: {err}")))?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
        };
        if let Some(quota) = quota(
            header("x-ratelimit-limit"),
            header("x-ratelimit-remaining"),
            header("x-ratelimit-reset"),
        ) {
            *self.quota.lock().unwrap_or_else(PoisonError::into_inner) = Some(quota);
        }
        let status = response.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(response.into_body());
        }
        let now = hldr_core::github::now();
        match RateLimit::from_answer(
            status,
            header("x-ratelimit-remaining"),
            header("x-ratelimit-reset"),
            header("retry-after"),
            now,
        ) {
            Some(limit) => Err(SyncError::RateLimited(format!(
                "{what}: {}",
                limit.message(now)
            ))),
            None => Err(SyncError::Fetch(format!("{what}: http status: {status}"))),
        }
    }

    fn get(&self, url: String) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let request = self
            .agent
            .get(url)
            .header("X-GitHub-Api-Version", "2022-11-28");
        match &self.token {
            Some(token) => request.header("Authorization", format!("Bearer {token}")),
            None => request,
        }
    }

    /// The commit a branch or tag points at, so one sync reads one revision
    /// even if the branch moves meanwhile. Anonymous answers are cached by
    /// GitHub's CDN for up to a minute; a query parameter it has not seen
    /// makes it ask the origin, so a push shows at once.
    fn resolve(&self, git_ref: &str) -> Result<String, SyncError> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let what = format!("resolve {git_ref}");
        let request = self
            .get(format!("{}/commits/{git_ref}?fresh={nonce}", self.base))
            .header("Accept", "application/vnd.github.sha");
        let sha = self
            .call(request, &what)?
            .with_config()
            .limit(128)
            .read_to_string()
            .map_err(|err| SyncError::Fetch(format!("resolve {git_ref}: {err}")))?;
        let sha = sha.trim();
        if sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(sha.to_ascii_lowercase())
        } else {
            Err(SyncError::Fetch(format!(
                "resolve {git_ref}: not a commit id: {sha:?}"
            )))
        }
    }

    fn download(&self, sha: &str, dest: &Path) -> Result<(), SyncError> {
        let request = self.get(format!("{}/tarball/{sha}", self.base));
        let reader = self
            .call(request, &format!("download {sha}"))?
            .into_with_config()
            .limit(MAX_ARCHIVE_BYTES)
            .reader();
        extract(flate2::read::GzDecoder::new(reader), dest)
            .map_err(|err| SyncError::Fetch(format!("unpack {sha}: {err}")))
    }
}

/// The rate limit an answer reports, when it reports one whole: the
/// tarball's redirect target, for one, does not.
fn quota(limit: Option<&str>, remaining: Option<&str>, reset: Option<&str>) -> Option<GitHubQuota> {
    let number = |value: Option<&str>| value?.trim().parse::<u64>().ok();
    Some(GitHubQuota {
        limit: number(limit)?,
        remaining: number(remaining)?,
        reset_at: store::from_unix(i64::try_from(number(reset)?).ok()?)?,
        observed_at: store::now_rfc3339(),
    })
}

/// Unpacks a GitHub tarball into `dest`, dropping the single top-level
/// directory GitHub wraps the tree in. Accepts only regular files and
/// directories inside `dest`: links, devices and escaping paths are errors,
/// as are archives past the size and entry budgets.
pub fn extract(archive: impl Read, dest: &Path) -> io::Result<()> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidData, message);
    let mut archive = tar::Archive::new(archive);
    let mut total: u64 = 0;
    for (index, entry) in archive.entries()?.enumerate() {
        if index >= MAX_ENTRIES {
            return Err(invalid(format!("more than {MAX_ENTRIES} entries")));
        }
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if kind == tar::EntryType::XGlobalHeader {
            continue;
        }
        let path = entry.path()?.into_owned();
        let mut components = path.components();
        if !matches!(components.next(), Some(Component::Normal(_))) {
            return Err(invalid(format!(
                "{}: outside the archive root",
                path.display()
            )));
        }
        let rel = components.as_path();
        if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(invalid(format!(
                "{}: escapes the archive root",
                path.display()
            )));
        }
        let target = dest.join(rel);
        match kind {
            tar::EntryType::Directory => fs::create_dir_all(&target)?,
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                if rel.as_os_str().is_empty() {
                    return Err(invalid(format!("{}: file at the root", path.display())));
                }
                total = total.saturating_add(entry.size());
                if total > MAX_CONTENT_BYTES {
                    return Err(invalid(format!("more than {MAX_CONTENT_BYTES} bytes")));
                }
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                let size = entry.size();
                let copied = io::copy(&mut (&mut entry).take(size), &mut File::create(&target)?)?;
                if copied != size {
                    return Err(invalid(format!("{}: truncated", path.display())));
                }
            }
            other => {
                return Err(invalid(format!(
                    "{}: unsupported entry type {other:?}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "hvpaiva-hldr-content-abc1234";

    fn raw_entry(
        builder: &mut tar::Builder<Vec<u8>>,
        name: &str,
        kind: tar::EntryType,
        data: &[u8],
    ) {
        let mut header = tar::Header::new_gnu();
        let field = &mut header.as_gnu_mut().unwrap().name;
        field.fill(0);
        field[..name.len()].copy_from_slice(name.as_bytes());
        header.set_entry_type(kind);
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, data).unwrap();
    }

    fn archive(entries: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, kind, data) in entries {
            raw_entry(&mut builder, name, *kind, data);
        }
        builder.into_inner().unwrap()
    }

    fn unpack(entries: &[(&str, tar::EntryType, &[u8])]) -> (tempfile::TempDir, io::Result<()>) {
        let dir = tempfile::tempdir().unwrap();
        let result = extract(archive(entries).as_slice(), dir.path());
        (dir, result)
    }

    #[test]
    fn unpacks_under_the_wrapping_directory() {
        use tar::EntryType::{Directory, Regular, XGlobalHeader};
        let (dir, result) = unpack(&[
            ("pax_global_header", XGlobalHeader, b"52 comment=abc\n"),
            (&format!("{ROOT}/"), Directory, b""),
            (&format!("{ROOT}/site.yaml"), Regular, b"kind: Site\n"),
            (&format!("{ROOT}/projects/atlas.md"), Regular, b"---\n"),
        ]);
        result.unwrap();
        assert_eq!(
            fs::read(dir.path().join("site.yaml")).unwrap(),
            b"kind: Site\n"
        );
        assert!(dir.path().join("projects/atlas.md").is_file());
    }

    #[test]
    fn rejects_paths_that_escape() {
        use tar::EntryType::Regular;
        for name in [
            format!("{ROOT}/../evil"),
            format!("{ROOT}/projects/../../evil"),
            "/etc/evil".to_owned(),
            "../evil".to_owned(),
            "top-level-file".to_owned(),
        ] {
            let (dir, result) = unpack(&[(&name, Regular, b"x")]);
            assert!(result.is_err(), "{name}");
            assert!(!dir.path().join("evil").exists(), "{name}");
        }
    }

    #[test]
    fn rejects_links_and_devices() {
        use tar::EntryType::{Block, Link, Symlink};
        for kind in [Symlink, Link, Block] {
            let (_dir, result) = unpack(&[(&format!("{ROOT}/site.yaml"), kind, b"")]);
            let err = result.unwrap_err();
            assert!(err.to_string().contains("unsupported"), "{kind:?}: {err}");
        }
    }

    #[test]
    fn rejects_oversized_content() {
        // Only the header: the budget is checked before any data is read.
        let mut header = tar::Header::new_gnu();
        header.set_path(format!("{ROOT}/big")).unwrap();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(MAX_CONTENT_BYTES + 1);
        header.set_cksum();
        let dir = tempfile::tempdir().unwrap();
        let err = extract(header.as_bytes().as_slice(), dir.path()).unwrap_err();
        assert!(err.to_string().contains("bytes"), "{err}");
    }

    #[test]
    fn parses_poll_intervals() {
        assert_eq!(
            parse_interval("30s").unwrap(),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            parse_interval("5m").unwrap(),
            Some(Duration::from_secs(300))
        );
        assert_eq!(
            parse_interval("1h").unwrap(),
            Some(Duration::from_secs(3600))
        );
        assert_eq!(parse_interval("0").unwrap(), None);
        assert_eq!(parse_interval("0m").unwrap(), None);
        for bad in ["", "5", "m", "5d", "-5m", "5 m"] {
            assert!(parse_interval(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn validates_repository_and_ref() {
        let ok = |repo, git_ref| ContentSource::github(GITHUB_API, repo, git_ref).is_ok();
        assert!(ok("hvpaiva/hldr-content", "main"));
        assert!(ok("hvpaiva/hldr.content", "release/v1"));
        assert!(!ok("hvpaiva", "main"));
        assert!(!ok("hvpaiva/hldr/content", "main"));
        assert!(!ok("hvpaiva/..", "main"));
        assert!(!ok("hvpaiva/x?y", "main"));
        assert!(!ok("hvpaiva/hldr-content", "../main"));
        assert!(!ok("hvpaiva/hldr-content", "main?x=1"));
        assert!(!ok("hvpaiva/hldr-content", ""));
    }

    /// A stand-in for the two GitHub endpoints the syncer calls.
    mod github_stub {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        use axum::Router;
        use axum::extract::{Path, State};
        use axum::http::StatusCode;
        use axum::response::{IntoResponse, Response};
        use axum::routing::get;

        #[derive(Clone, Default)]
        pub struct Stub {
            pub sha: Arc<Mutex<Option<String>>>,
            pub downloads: Arc<AtomicUsize>,
            /// When set, every answer is a spent rate limit resetting then.
            pub limited_until: Arc<Mutex<Option<u64>>>,
        }

        impl Stub {
            pub fn point_at(&self, sha: Option<&str>) {
                *self.sha.lock().unwrap() = sha.map(str::to_owned);
            }

            pub fn downloads(&self) -> usize {
                self.downloads.load(Ordering::SeqCst)
            }

            pub async fn serve(&self) -> String {
                let app = Router::new()
                    .route("/repos/o/r/commits/main", get(commit))
                    .route("/repos/o/r/tarball/{sha}", get(tarball))
                    .with_state(self.clone());
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
                format!("http://{addr}")
            }
        }

        /// Resets at 2026-09-22T12:00:00Z.
        pub const RESET: u64 = 1_790_078_400;

        async fn commit(State(stub): State<Stub>) -> Response {
            if let Some(reset) = *stub.limited_until.lock().unwrap() {
                let headers = [
                    ("x-ratelimit-limit", "60".to_owned()),
                    ("x-ratelimit-remaining", "0".to_owned()),
                    ("x-ratelimit-reset", reset.to_string()),
                ];
                return (StatusCode::FORBIDDEN, headers, "rate limited").into_response();
            }
            let headers = [
                ("x-ratelimit-limit", "60".to_owned()),
                ("x-ratelimit-remaining", "59".to_owned()),
                ("x-ratelimit-reset", RESET.to_string()),
            ];
            match stub.sha.lock().unwrap().clone() {
                Some(sha) => (headers, sha).into_response(),
                None => (StatusCode::NOT_FOUND, headers).into_response(),
            }
        }

        async fn tarball(
            State(stub): State<Stub>,
            Path(sha): Path<String>,
        ) -> Result<Vec<u8>, StatusCode> {
            if stub.sha.lock().unwrap().as_deref() != Some(sha.as_str()) {
                return Err(StatusCode::NOT_FOUND);
            }
            stub.downloads.fetch_add(1, Ordering::SeqCst);
            let fixtures =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/content");
            let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            let mut builder = tar::Builder::new(gz);
            builder
                .append_dir_all(format!("o-r-{}", &sha[..7]), fixtures)
                .unwrap();
            Ok(builder.into_inner().unwrap().finish().unwrap())
        }
    }

    #[tokio::test]
    async fn syncs_from_github_by_revision() {
        let first = "a".repeat(40);
        let second = "b".repeat(40);
        let stub = github_stub::Stub::default();
        stub.point_at(Some(&first));
        let api = stub.serve().await;

        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        let source = ContentSource::github(&api, "o/r", "main").unwrap();
        let syncer = Syncer::new(source, db.clone());

        let report = syncer.sync(true).await.unwrap().unwrap();
        assert_eq!(report.get("Project").upserted, 2);
        assert_eq!(db.sync_state().await.unwrap().revision, Some(first.clone()));
        assert_eq!(
            db.page("Project", "atlas")
                .await
                .unwrap()
                .unwrap()
                .page
                .title(),
            "Atlas"
        );

        assert!(syncer.sync(false).await.unwrap().is_none());
        assert_eq!(stub.downloads(), 1);

        stub.point_at(Some(&second));
        assert!(syncer.sync(false).await.unwrap().is_some());
        assert_eq!(stub.downloads(), 2);
        assert_eq!(
            db.sync_state().await.unwrap().revision,
            Some(second.clone())
        );

        stub.point_at(None);
        let err = syncer.sync(false).await.unwrap_err();
        assert!(matches!(err, SyncError::Fetch(_)), "{err}");
        let state = db.sync_state().await.unwrap();
        assert_eq!(state.revision, Some(second.clone()));
        assert!(state.last_error.unwrap().contains("404"));
        syncer.sync(false).await.unwrap_err();

        let events: Vec<_> = db
            .events(None)
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|event| (event.reason, event.message, event.revision, event.count))
            .collect();
        assert_eq!(
            events,
            [
                (
                    "Synced".to_owned(),
                    "nothing → aaaaaaaaaaaa: Collection 2 upserted, Nav 1 upserted, \
                     Page 4 upserted, PageKind 2 upserted, Profile 1 upserted, \
                     Project 2 upserted, Site 1 upserted, Theme 2 upserted"
                        .to_owned(),
                    Some(first),
                    1
                ),
                (
                    "Synced".to_owned(),
                    "aaaaaaaaaaaa → bbbbbbbbbbbb: no resource changed".to_owned(),
                    Some(second.clone()),
                    1
                ),
                (
                    "SyncFailed".to_owned(),
                    "fetch: resolve main: http status: 404".to_owned(),
                    Some(second),
                    2
                ),
            ]
        );
        let quota = syncer.quota().unwrap();
        assert_eq!((quota.limit, quota.remaining), (60, 59));
        assert_eq!(quota.reset_at, "2026-09-22T12:00:00Z");
    }

    #[tokio::test]
    async fn a_spent_rate_limit_is_recorded_with_its_reset() {
        let stub = github_stub::Stub::default();
        *stub.limited_until.lock().unwrap() = Some(hldr_core::github::now() + 1800);
        let api = stub.serve().await;
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        let syncer = Syncer::new(
            ContentSource::github(&api, "o/r", "main").unwrap(),
            db.clone(),
        );

        let err = syncer.sync(false).await.unwrap_err();
        assert!(matches!(err, SyncError::RateLimited(_)), "{err}");
        let recorded = db.sync_state().await.unwrap().last_error.unwrap();
        assert!(
            recorded.starts_with("fetch: resolve main: GitHub rate limit exceeded; it resets at "),
            "{recorded}"
        );
        assert!(recorded.ends_with("in 30 min"), "{recorded}");
        let events = db.events(None).await.unwrap().items;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reason, "RateLimited");
        assert_eq!(events[0].message, recorded);
        assert_eq!(syncer.quota().unwrap().remaining, 0);
    }

    #[tokio::test]
    async fn concurrent_syncs_run_one_at_a_time() {
        let stub = github_stub::Stub::default();
        stub.point_at(Some(&"c".repeat(40)));
        let api = stub.serve().await;
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        let syncer = std::sync::Arc::new(Syncer::new(
            ContentSource::github(&api, "o/r", "main").unwrap(),
            db,
        ));

        let runs = (0..4).map(|_| {
            let syncer = std::sync::Arc::clone(&syncer);
            tokio::spawn(async move { syncer.sync(false).await.unwrap().is_some() })
        });
        let mut indexed = 0;
        for run in runs {
            indexed += usize::from(run.await.unwrap());
        }
        assert_eq!(indexed, 1);
        assert_eq!(stub.downloads(), 1);
    }

    #[tokio::test]
    async fn waits_for_the_revision_a_client_pushed() {
        let old = "d".repeat(40);
        let new = "e".repeat(40);
        let stub = github_stub::Stub::default();
        stub.point_at(Some(&old));
        let api = stub.serve().await;
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        let syncer = std::sync::Arc::new(
            Syncer::new(
                ContentSource::github(&api, "o/r", "main").unwrap(),
                db.clone(),
            )
            .with_patience(Duration::from_millis(20), Duration::from_secs(5)),
        );
        syncer.sync(true).await.unwrap();

        let waiting = {
            let syncer = std::sync::Arc::clone(&syncer);
            let new = new.clone();
            tokio::spawn(async move { syncer.sync_to(false, Some(&new)).await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        stub.point_at(Some(&new));
        assert!(waiting.await.unwrap().unwrap().is_some());
        assert_eq!(db.sync_state().await.unwrap().revision, Some(new));
    }

    #[tokio::test]
    async fn gives_up_on_a_revision_that_never_arrives() {
        let stub = github_stub::Stub::default();
        stub.point_at(Some(&"d".repeat(40)));
        let api = stub.serve().await;
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        let syncer = Syncer::new(ContentSource::github(&api, "o/r", "main").unwrap(), db)
            .with_patience(Duration::from_millis(10), Duration::from_millis(50));
        let err = syncer
            .sync_to(false, Some(&"f".repeat(40)))
            .await
            .unwrap_err();
        assert!(matches!(err, SyncError::Stale(_)), "{err}");
        let err = syncer.sync_to(false, Some("nope")).await.unwrap_err();
        assert!(err.to_string().contains("not a commit id"), "{err}");
    }
}
