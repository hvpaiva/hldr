//! The metrics backup: every closed day, as one file in a private GitHub
//! repository, so losing the database loses at most the day in progress.
//!
//! A pass restores the repository into a database that was never backed
//! up, then uploads the closed days the repository lacks, oldest first.
//! Each upload reads the file first: the same bytes are not sent again, so
//! the two servers of a deploy can both pass. A failed pass is a warning
//! event and is retried an hour later. Only counts travel: a closed day
//! holds no hash, salt, address or user agent.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hldr_core::Db;
use hldr_core::api::BackupState;
use hldr_core::backup::DayRecord;
use hldr_core::events::NewEvent;
use hldr_core::store;
use serde_json::{Value, json};

use crate::content;

/// How often a pass runs: soon after the flush that closes a day.
pub const PASS_EVERY: Duration = Duration::from_secs(60);

/// How long a failed pass waits before the next try.
const RETRY_AFTER: Duration = Duration::from_secs(3600);

/// From how many days before the token expires a warning is written, once
/// a day.
const EXPIRY_WARNING_DAYS: i64 = 30;

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

pub struct Backup {
    db: Db,
    repo: Repo,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Set once the repository was restored, or needed no restore.
    restored: bool,
    failed_at: Option<Instant>,
    last_error: Option<String>,
    /// The day the last expiry warning was written.
    warned_on: Option<String>,
}

/// Why a pass stopped: the event's reason and message.
struct Failure(&'static str, String);

impl Backup {
    /// The backup `HLDR_METRICS_REPO` and `HLDR_METRICS_TOKEN` configure;
    /// none when either is empty, as in development and before the token
    /// exists.
    pub fn from_env(db: Db) -> Result<Option<Self>, String> {
        let var = |name| std::env::var(name).unwrap_or_default().trim().to_owned();
        let (repository, token) = (var("HLDR_METRICS_REPO"), var("HLDR_METRICS_TOKEN"));
        match (repository.is_empty(), token.is_empty()) {
            (true, _) => Ok(None),
            (false, true) => {
                tracing::info!(%repository, "metrics backup off: HLDR_METRICS_TOKEN is empty");
                Ok(None)
            }
            (false, false) => Self::new(content::GITHUB_API, &repository, token, db).map(Some),
        }
    }

    pub fn new(api: &str, repository: &str, token: String, db: Db) -> Result<Self, String> {
        if !is_repository(repository) {
            return Err(format!(
                "HLDR_METRICS_REPO must be owner/name, not {repository:?}"
            ));
        }
        Ok(Self {
            db,
            repo: Repo::new(api, repository, token),
            state: Mutex::default(),
        })
    }

    /// What `describe server` shows.
    pub async fn state(&self) -> Result<BackupState, hldr_core::Error> {
        let pending = self.db.pending_backups().await?.len();
        let last_error = lock(&self.state).last_error.clone();
        Ok(BackupState {
            repository: self.repo.name.clone(),
            last_day: self.db.last_backup().await?,
            pending: u64::try_from(pending).unwrap_or(u64::MAX),
            token_expires_at: self.repo.expiry(),
            last_error,
        })
    }

    /// One pass, unless the last one failed less than an hour ago.
    pub async fn pass(&self) {
        let waiting = lock(&self.state)
            .failed_at
            .is_some_and(|failed| failed.elapsed() < RETRY_AFTER);
        if !waiting {
            let failure = self.try_pass().await.err();
            {
                let mut state = lock(&self.state);
                state.failed_at = failure.as_ref().map(|_| Instant::now());
                state.last_error = failure.as_ref().map(|Failure(_, message)| message.clone());
            }
            if let Some(Failure(reason, message)) = failure {
                tracing::warn!(reason, %message, "metrics backup failed");
                content::record(&self.db, &NewEvent::warning(reason, message)).await;
            }
        }
        self.warn_of_expiry().await;
    }

    async fn try_pass(&self) -> Result<(), Failure> {
        let failed = |reason| move |message: String| Failure(reason, message);
        if !lock(&self.state).restored {
            if !self
                .db
                .has_backups()
                .await
                .map_err(|err| Failure("RestoreFailed", err.to_string()))?
            {
                self.restore().await.map_err(failed("RestoreFailed"))?;
            }
            lock(&self.state).restored = true;
        }
        let pending = self
            .db
            .pending_backups()
            .await
            .map_err(|err| Failure("BackupFailed", err.to_string()))?;
        let mut uploaded = Vec::new();
        let mut result = Ok(());
        for day in pending {
            if let Err(message) = self.upload(&day).await {
                result = Err(Failure("BackupFailed", format!("{day}: {message}")));
                break;
            }
            uploaded.push(day);
        }
        if let (Some(first), Some(last)) = (uploaded.first(), uploaded.last()) {
            let days = match uploaded.len() {
                1 => first.clone(),
                n => format!("{n} days, {first} to {last}"),
            };
            let message = format!("metrics for {days} backed up to {}", self.repo.name);
            content::record(&self.db, &NewEvent::normal("BackedUp", message)).await;
        }
        result
    }

    async fn upload(&self, day: &str) -> Result<(), String> {
        let record = self
            .db
            .day_record(day)
            .await
            .map_err(|err| err.to_string())?
            .ok_or("the day is not closed")?;
        let path = record.path().map_err(|err| err.to_string())?;
        let bytes = record.to_bytes().map_err(|err| err.to_string())?;
        let repo = self.repo.clone();
        let message = format!("metrics: {day}");
        let sha = blocking(move || repo.write(&path, &bytes, &message)).await?;
        self.db
            .record_backup(day, &sha)
            .await
            .map_err(|err| err.to_string())
    }

    /// Loads every day the repository holds; the event says how many.
    async fn restore(&self) -> Result<(), String> {
        let repo = self.repo.clone();
        let records = blocking(move || repo.days()).await?;
        let restored = self
            .db
            .restore_days(&records)
            .await
            .map_err(|err| err.to_string())?;
        tracing::info!(restored, repository = %self.repo.name, "metrics restored");
        if restored > 0 {
            let message = format!(
                "metrics for {restored} days restored from {}",
                self.repo.name
            );
            content::record(&self.db, &NewEvent::normal("Restored", message)).await;
        }
        Ok(())
    }

    /// A warning a day once the token has less than a month left.
    async fn warn_of_expiry(&self) {
        let Some(expiry) = self.repo.expiry() else {
            return;
        };
        if !store::within_days(&expiry, EXPIRY_WARNING_DAYS) {
            return;
        }
        let today = hldr_core::metrics::today();
        {
            let mut state = lock(&self.state);
            if state.warned_on.as_deref() == Some(today.as_str()) {
                return;
            }
            state.warned_on = Some(today);
        }
        let message = format!(
            "the token for {} expires at {expiry}; replace HLDR_METRICS_TOKEN",
            self.repo.name
        );
        content::record(&self.db, &NewEvent::warning("TokenExpiring", message)).await;
    }
}

/// Passes every `every`, the first at once: a fresh database is restored
/// right after boot.
pub async fn run_every(backup: Arc<Backup>, every: Duration) {
    let mut ticker = tokio::time::interval(every);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        backup.pass().await;
    }
}

/// `owner/name`, each of letters, digits, `-`, `_` and `.`, and neither `.`
/// nor `..`.
fn is_repository(text: &str) -> bool {
    text.split_once('/').is_some_and(|(owner, name)| {
        [owner, name].iter().all(|part| {
            !matches!(*part, "" | "." | "..")
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| err.to_string())?
}

/// The metrics repository through the GitHub REST API, with the token.
#[derive(Clone)]
struct Repo {
    agent: ureq::Agent,
    base: String,
    name: String,
    token: String,
    expiry: Arc<Mutex<Option<String>>>,
}

impl Repo {
    fn new(api: &str, name: &str, token: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .http_status_as_error(false)
            .user_agent(format!("hldr-server/{}", hldr_core::VERSION))
            .build()
            .into();
        Self {
            agent,
            base: format!("{api}/repos/{name}"),
            name: name.to_owned(),
            token,
            expiry: Arc::default(),
        }
    }

    fn expiry(&self) -> Option<String> {
        lock(&self.expiry).clone()
    }

    fn authorized<B>(&self, request: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        request
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    /// The status and body of an answer, noting the token's expiry when
    /// GitHub reports it.
    fn answer(
        &self,
        response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
        what: &str,
    ) -> Result<(u16, ureq::Body), String> {
        let response = response.map_err(|err| format!("{what}: {err}"))?;
        if let Some(expiry) = response
            .headers()
            .get("github-authentication-token-expiration")
            .and_then(|value| value.to_str().ok())
            .and_then(store::from_github_time)
        {
            *lock(&self.expiry) = Some(expiry);
        }
        Ok((response.status().as_u16(), response.into_body()))
    }

    fn json(body: ureq::Body, what: &str) -> Result<Value, String> {
        let text = body
            .into_with_config()
            .limit(MAX_FILE_BYTES * 2)
            .read_to_string()
            .map_err(|err| format!("{what}: {err}"))?;
        serde_json::from_str(&text).map_err(|err| format!("{what}: {err}"))
    }

    /// The blob id and bytes of a file, or none when it does not exist.
    fn read(&self, path: &str) -> Result<Option<(String, Vec<u8>)>, String> {
        let what = format!("read {path}");
        let request = self.authorized(self.agent.get(format!("{}/contents/{path}", self.base)));
        match self.answer(request.call(), &what)? {
            (404, _) => Ok(None),
            (200, body) => {
                let file = Self::json(body, &what)?;
                let sha = file["sha"].as_str().ok_or(format!("{what}: no sha"))?;
                let encoded: String = file["content"]
                    .as_str()
                    .ok_or(format!("{what}: no content"))?
                    .split_whitespace()
                    .collect();
                let bytes = STANDARD
                    .decode(encoded)
                    .map_err(|err| format!("{what}: {err}"))?;
                Ok(Some((sha.to_owned(), bytes)))
            }
            (status, _) => Err(format!("{what}: http status: {status}")),
        }
    }

    /// Makes `path` hold `bytes`, answering the file's blob id. The same
    /// bytes are left alone; a write that lost a race with the other
    /// server of a deploy reads again once.
    fn write(&self, path: &str, bytes: &[u8], message: &str) -> Result<String, String> {
        let what = format!("write {path}");
        for attempt in 0..2 {
            let existing = self.read(path)?;
            if let Some((sha, current)) = &existing
                && current == bytes
            {
                return Ok(sha.clone());
            }
            let mut body = json!({"message": message, "content": STANDARD.encode(bytes)});
            if let Some((sha, _)) = &existing {
                body["sha"] = json!(sha);
            }
            let request = self
                .authorized(self.agent.put(format!("{}/contents/{path}", self.base)))
                .header("Content-Type", "application/json");
            match self.answer(request.send(body.to_string()), &what)? {
                (200 | 201, body) => {
                    return Self::json(body, &what)?["content"]["sha"]
                        .as_str()
                        .map(str::to_owned)
                        .ok_or(format!("{what}: no sha"));
                }
                (409 | 422, _) if attempt == 0 => continue,
                (status, _) => return Err(format!("{what}: http status: {status}")),
            }
        }
        Err(format!("{what}: changed while being written"))
    }

    /// Every day file of the default branch, from one tarball. A repository
    /// without commits holds none.
    fn days(&self) -> Result<Vec<DayRecord>, String> {
        let what = "download";
        let request = self.authorized(self.agent.get(format!("{}/tarball", self.base)));
        let body = match self.answer(request.call(), what)? {
            (200, body) => body,
            (404, _) if self.exists()? => return Ok(Vec::new()),
            (status, _) => return Err(format!("{what}: http status: {status}")),
        };
        let dir = tempfile::tempdir().map_err(|err| format!("{what}: {err}"))?;
        let reader = body.into_with_config().limit(MAX_ARCHIVE_BYTES).reader();
        content::extract(flate2::read::GzDecoder::new(reader), dir.path())
            .map_err(|err| format!("unpack: {err}"))?;
        let mut records = Vec::new();
        collect(dir.path(), &dir.path().join("days"), &mut records)?;
        records.sort_by(|a, b| a.day.cmp(&b.day));
        Ok(records)
    }

    fn exists(&self) -> Result<bool, String> {
        let request = self.authorized(self.agent.get(self.base.clone()));
        match self.answer(request.call(), "read the repository")? {
            (200, _) => Ok(true),
            (404, _) => Ok(false),
            (status, _) => Err(format!("read the repository: http status: {status}")),
        }
    }
}

/// The day files under `dir`, named by their path from `root`.
fn collect(root: &Path, dir: &Path, records: &mut Vec<DayRecord>) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("{}: {err}", dir.display())),
    };
    for entry in entries {
        let path = entry.map_err(|err| err.to_string())?.path();
        if path.is_dir() {
            collect(root, &path, records)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let name = path
                .strip_prefix(root)
                .map_err(|err| err.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&path).map_err(|err| format!("{name}: {err}"))?;
            records.push(DayRecord::from_file(&name, &bytes).map_err(|err| err.to_string())?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use axum::Router;
    use axum::extract::{Path as UrlPath, State};
    use axum::http::{StatusCode, header};
    use axum::response::{IntoResponse, Response};
    use axum::routing::get;
    use hldr_core::backup::PageCounts;
    use hldr_core::metrics::Batch;

    use super::*;

    const EXPIRY: &str = "2026-10-01 00:00:00 UTC";

    /// Path to (blob id, bytes).
    type Files = BTreeMap<String, (String, Vec<u8>)>;

    /// The metrics repository in memory.
    #[derive(Clone, Default)]
    struct Hub {
        files: Arc<Mutex<Files>>,
        writes: Arc<Mutex<usize>>,
        broken: Arc<Mutex<bool>>,
    }

    impl Hub {
        async fn serve(&self) -> String {
            let app = Router::new()
                .route("/repos/o/metrics", get(|| async { "{}" }))
                .route("/repos/o/metrics/tarball", get(tarball))
                .route(
                    "/repos/o/metrics/contents/{*path}",
                    get(read_file).put(write_file),
                )
                .layer(axum::middleware::map_response(
                    |mut response: Response| async move {
                        response.headers_mut().insert(
                            "github-authentication-token-expiration",
                            header::HeaderValue::from_static(EXPIRY),
                        );
                        response
                    },
                ))
                .with_state(self.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            base
        }

        fn file(&self, path: &str) -> Option<String> {
            let files = self.files.lock().unwrap();
            files
                .get(path)
                .map(|(_, bytes)| String::from_utf8(bytes.clone()).unwrap())
        }

        fn writes(&self) -> usize {
            *self.writes.lock().unwrap()
        }
    }

    async fn read_file(State(hub): State<Hub>, UrlPath(path): UrlPath<String>) -> Response {
        match hub.files.lock().unwrap().get(&path) {
            Some((sha, bytes)) => axum::Json(json!({
                "sha": sha,
                "encoding": "base64",
                "content": STANDARD.encode(bytes),
            }))
            .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    async fn write_file(
        State(hub): State<Hub>,
        UrlPath(path): UrlPath<String>,
        axum::Json(body): axum::Json<Value>,
    ) -> Response {
        if *hub.broken.lock().unwrap() {
            return StatusCode::FORBIDDEN.into_response();
        }
        let mut files = hub.files.lock().unwrap();
        let current = files.get(&path).map(|(sha, _)| sha.clone());
        if body["sha"].as_str() != current.as_deref() {
            return StatusCode::CONFLICT.into_response();
        }
        let mut writes = hub.writes.lock().unwrap();
        *writes += 1;
        let sha = format!("blob-{writes}");
        let bytes = STANDARD.decode(body["content"].as_str().unwrap()).unwrap();
        files.insert(path, (sha.clone(), bytes));
        (
            StatusCode::CREATED,
            axum::Json(json!({"content": {"sha": sha}})),
        )
            .into_response()
    }

    async fn tarball(State(hub): State<Hub>) -> Response {
        let files = hub.files.lock().unwrap().clone();
        if files.is_empty() {
            return StatusCode::NOT_FOUND.into_response();
        }
        let mut archive = tar::Builder::new(Vec::new());
        for (path, (_, bytes)) in files {
            let mut head = tar::Header::new_gnu();
            head.set_size(bytes.len() as u64);
            head.set_mode(0o644);
            head.set_cksum();
            archive
                .append_data(&mut head, format!("o-metrics-abc/{path}"), bytes.as_slice())
                .unwrap();
        }
        let tar = archive.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar).unwrap();
        gz.finish().unwrap().into_response()
    }

    async fn db(dir: &tempfile::TempDir, name: &str) -> Db {
        Db::open(dir.path().join(name)).await.unwrap()
    }

    /// A day long closed: one visitor viewed `/` twice, one bot came.
    async fn closed_day(db: &Db, day: &str) {
        let mut batch = Batch::default();
        batch.views.insert((day.to_owned(), "/".to_owned()), 2);
        batch
            .visitors
            .insert((day.to_owned(), "/".to_owned(), [9; 32]));
        batch.bots.insert(day.to_owned(), 1);
        db.flush_metrics(&batch).await.unwrap();
    }

    async fn reasons(db: &Db) -> Vec<(String, String)> {
        db.events(None)
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|event| (event.reason, event.message))
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn uploads_closed_days_once_and_restores_them() {
        let hub = Hub::default();
        let api = hub.serve().await;
        let dir = tempfile::tempdir().unwrap();
        let db = db(&dir, "one.db").await;
        let backup = Backup::new(&api, "o/metrics", "t".to_owned(), db.clone()).unwrap();

        backup.pass().await;
        assert_eq!(hub.writes(), 0, "an empty repository restores nothing");
        closed_day(&db, "2020-01-01").await;
        closed_day(&db, "2020-01-02").await;
        backup.pass().await;
        assert_eq!(hub.writes(), 2);
        let file = hub.file("days/2020/01/2020-01-01.json").unwrap();
        assert!(file.contains("\"visitors\": 1"), "{file}");
        let state = backup.state().await.unwrap();
        assert_eq!(state.last_day.as_deref(), Some("2020-01-02"));
        assert_eq!(state.pending, 0);
        assert_eq!(
            state.token_expires_at.as_deref(),
            Some("2026-10-01T00:00:00Z")
        );
        assert!(state.last_error.is_none());

        // The other server of a deploy, on the same database, or a later
        // pass of this one: nothing to send, and the same bytes are not sent.
        let other = Backup::new(&api, "o/metrics", "t".to_owned(), db.clone()).unwrap();
        other.pass().await;
        sqlx::query("DELETE FROM metrics_backup")
            .execute(db.pool())
            .await
            .unwrap();
        lock(&other.state).restored = true;
        other.pass().await;
        assert_eq!(hub.writes(), 2, "unchanged files are not written again");

        let fresh = self::db(&dir, "fresh.db").await;
        Backup::new(&api, "o/metrics", "t".to_owned(), fresh.clone())
            .unwrap()
            .pass()
            .await;
        let restored = fresh.day_record("2020-01-01").await.unwrap().unwrap();
        assert_eq!(
            restored.pages["/"],
            PageCounts {
                views: 2,
                visitors: 1
            }
        );
        assert!(fresh.pending_backups().await.unwrap().is_empty());
        assert_eq!(
            reasons(&fresh).await[0],
            (
                "Restored".to_owned(),
                "metrics for 2 days restored from o/metrics".to_owned()
            )
        );
        let events = reasons(&db).await;
        assert!(events.iter().any(|(reason, message)| reason == "BackedUp"
            && message == "metrics for 2 days, 2020-01-01 to 2020-01-02 backed up to o/metrics"));
        assert!(
            events.iter().any(|(reason, _)| reason == "TokenExpiring"),
            "{events:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_upload_warns_and_waits_an_hour() {
        let hub = Hub::default();
        let api = hub.serve().await;
        let dir = tempfile::tempdir().unwrap();
        let db = db(&dir, "one.db").await;
        let backup = Backup::new(&api, "o/metrics", "t".to_owned(), db.clone()).unwrap();
        closed_day(&db, "2020-01-01").await;
        *hub.broken.lock().unwrap() = true;
        backup.pass().await;
        let state = backup.state().await.unwrap();
        assert_eq!(state.pending, 1);
        let error = state.last_error.unwrap();
        assert!(error.contains("http status: 403"), "{error}");
        assert!(
            reasons(&db)
                .await
                .iter()
                .any(|(reason, _)| reason == "BackupFailed")
        );

        *hub.broken.lock().unwrap() = false;
        backup.pass().await;
        assert_eq!(hub.writes(), 0, "retried only an hour later");
        lock(&backup.state).failed_at = None;
        backup.pass().await;
        assert_eq!(hub.writes(), 1);
        assert!(backup.state().await.unwrap().last_error.is_none());
    }

    #[test]
    fn the_repository_is_owner_and_name() {
        assert!(is_repository("hvpaiva/hldr-metrics"));
        assert!(is_repository("a.b/c_d"));
        for bad in [
            "hvpaiva", "/x", "a/", "a/b/c", "a/b c", "a/..", "./b", "a/b?x",
        ] {
            assert!(!is_repository(bad), "{bad:?}");
        }
    }
}
