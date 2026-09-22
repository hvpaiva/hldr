//! GitHub access for writing content: the hldr GitHub App's device flow,
//! or a token pasted in, kept in `$XDG_STATE_HOME/hldr/credentials.json`.
//!
//! The app is installed on the content repository alone, with Contents read
//! and write, so what it grants reaches that repository and nothing else. Its
//! user access token lives eight hours; the refresh token that renews it
//! lives six months and is spent on use, GitHub answering with a new pair.
//! The CLI renews an hour ahead, so a run that keeps the editor open for a
//! while still writes, and under a lock, so two runs never spend the same
//! refresh token.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::color::Term;

/// The hldr GitHub App. Its client ID is public: the device flow needs no
/// secret, and neither does renewing what the device flow granted.
pub const CLIENT_ID: &str = "Iv23liye1szfukuGjZRE";
const OAUTH: &str = "https://github.com";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// Renew once less than this is left, in seconds.
const RENEW_AHEAD: u64 = 3600;
/// Warn once a pasted token has less than this left, in seconds.
const TOKEN_WARNING: u64 = 14 * 86_400;
const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY: u64 = 1024 * 1024;
const FORMAT: u32 = 1;

/// Where to see and revoke what hldr holds, since revoking through the API
/// takes the app's secret, which the CLI does not have.
pub const APP_AUTHORIZATIONS: &str = "https://github.com/settings/apps/authorizations";
pub const TOKENS: &str = "https://github.com/settings/personal-access-tokens";
pub const INSTALLATIONS: &str = "https://github.com/settings/installations";

/// A stored GitHub credential. No `Debug`: it holds tokens.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "lowercase")]
pub enum Credential {
    /// From the app's device flow. Times are seconds since the Unix epoch;
    /// they are absent when the app's tokens do not expire.
    App {
        user: String,
        access_token: String,
        #[serde(default)]
        expires_at: Option<u64>,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        refresh_expires_at: Option<u64>,
    },
    /// Pasted in, such as a fine-grained personal access token; its expiry
    /// is what GitHub reported when it was stored.
    Token {
        user: String,
        token: String,
        #[serde(default)]
        expires_at: Option<u64>,
    },
}

impl Credential {
    pub fn user(&self) -> &str {
        match self {
            Self::App { user, .. } | Self::Token { user, .. } => user,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    #[serde(
        rename = "github.com",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    github: Option<Credential>,
}

/// The credential file and the lock beside it.
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join("credentials.json")
    }

    /// Held while the file is read and rewritten; released when dropped.
    fn lock(&self) -> Result<File> {
        self.create_dir()?;
        let path = self.dir.join("credentials.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("open {}", path.display()))?;
        file.lock()
            .with_context(|| format!("lock {}", path.display()))?;
        Ok(file)
    }

    fn create_dir(&self) -> Result<()> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)
            .with_context(|| format!("create {}", self.dir.display()))
    }

    /// The stored credential; `None` when there is none. A file others can
    /// read is refused, as ssh refuses such a key.
    pub fn load(&self) -> Result<Option<Credential>> {
        let path = self.path();
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).with_context(|| format!("open {}", path.display())),
        };
        let mode = file.metadata()?.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            bail!(
                "{} is open to other users (mode {mode:o}); run `chmod 600` on it",
                path.display()
            );
        }
        let mut text = String::new();
        file.read_to_string(&mut text)
            .with_context(|| format!("read {}", path.display()))?;
        let stored: Stored =
            serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        if stored.version != FORMAT {
            bail!(
                "{} has format {}, which this hldr does not read; run `hldr auth login`",
                path.display(),
                stored.version
            );
        }
        Ok(stored.github)
    }

    /// Replaces the file whole: a new file created `0600`, then renamed over
    /// the old one, so a crash leaves one or the other.
    fn save(&self, credential: &Credential) -> Result<()> {
        self.create_dir()?;
        let path = self.path();
        let tmp = self
            .dir
            .join(format!(".credentials.{}.tmp", std::process::id()));
        let json = serde_json::to_vec_pretty(&Stored {
            version: FORMAT,
            github: Some(credential.clone()),
        })?;
        let _ = std::fs::remove_file(&tmp);
        let written = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .and_then(|mut file| {
                file.write_all(&json)?;
                file.sync_all()
            })
            .and_then(|()| std::fs::rename(&tmp, &path));
        if let Err(err) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(err).with_context(|| format!("write {}", path.display()));
        }
        Ok(())
    }

    /// Stores `credential`, replacing any other.
    pub fn store(&self, credential: &Credential) -> Result<()> {
        let _lock = self.lock()?;
        self.save(credential)
    }

    /// Forgets the credential; whether there was one.
    pub fn remove(&self) -> Result<bool> {
        let _lock = self.lock()?;
        self.remove_locked()
    }

    fn remove_locked(&self) -> Result<bool> {
        match std::fs::remove_file(self.path()) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err).with_context(|| format!("remove {}", self.path().display())),
        }
    }
}

/// What GitHub shows to start the device flow.
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    device_code: String,
    expires_in: u64,
    interval: u64,
}

/// Tokens GitHub granted, with their lifetimes in seconds.
struct Grant {
    access_token: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    refresh_token_expires_in: Option<u64>,
}

impl Grant {
    fn from_answer(answer: &Value) -> Result<Self> {
        let access_token = answer["access_token"]
            .as_str()
            .filter(|token| !token.is_empty())
            .context("GitHub granted no access token")?
            .to_owned();
        Ok(Self {
            access_token,
            expires_in: answer["expires_in"].as_u64(),
            refresh_token: answer["refresh_token"].as_str().map(str::to_owned),
            refresh_token_expires_in: answer["refresh_token_expires_in"].as_u64(),
        })
    }

    fn credential(self, user: String, now: u64) -> Credential {
        Credential::App {
            user,
            access_token: self.access_token,
            expires_at: self.expires_in.map(|secs| now + secs),
            refresh_token: self.refresh_token,
            refresh_expires_at: self.refresh_token_expires_in.map(|secs| now + secs),
        }
    }
}

/// Why a refresh token did not renew the session.
enum RenewError {
    /// GitHub refused it: expired, revoked, or already spent.
    Refused(String),
    /// GitHub could not be asked.
    Unreachable(anyhow::Error),
}

/// GitHub's OAuth endpoints and the API calls that describe a token.
pub struct OAuth {
    agent: ureq::Agent,
    base: String,
    api: String,
    client_id: String,
}

impl OAuth {
    pub fn github() -> Self {
        Self::new(OAUTH, crate::github::API, CLIENT_ID)
    }

    pub fn new(base: &str, api: &str, client_id: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .user_agent(format!("hldr/{}", hldr_core::VERSION))
            .build()
            .into();
        Self {
            agent,
            base: base.trim_end_matches('/').to_owned(),
            api: api.trim_end_matches('/').to_owned(),
            client_id: client_id.to_owned(),
        }
    }

    /// Starts the device flow: a code for the user to enter at GitHub.
    pub fn start(&self) -> Result<DeviceCode> {
        let answer = self.form("/login/device/code", &[("client_id", &self.client_id)])?;
        if let Some(error) = answer["error"].as_str() {
            bail!(
                "GitHub refused to start the login: {}",
                described(&answer, error)
            );
        }
        let text = |field: &str| -> Result<String> {
            answer[field]
                .as_str()
                .map(str::to_owned)
                .with_context(|| format!("GitHub's device code answer lacks {field}"))
        };
        let verification_uri = text("verification_uri")?;
        if !verification_uri.starts_with("https://") {
            bail!("GitHub sent a verification address that is not https: {verification_uri}");
        }
        Ok(DeviceCode {
            user_code: text("user_code")?,
            verification_uri,
            device_code: text("device_code")?,
            expires_in: answer["expires_in"].as_u64().unwrap_or(900),
            interval: answer["interval"].as_u64().unwrap_or(5),
        })
    }

    /// Polls until the user enters the code, refuses, or it expires, at the
    /// pace GitHub sets.
    fn wait(
        &self,
        code: &DeviceCode,
        now: &dyn Fn() -> u64,
        sleep: &dyn Fn(Duration),
    ) -> Result<Grant> {
        let deadline = now() + code.expires_in;
        let mut interval = code.interval.max(1);
        loop {
            sleep(Duration::from_secs(interval));
            let answer = self.form(
                "/login/oauth/access_token",
                &[
                    ("client_id", &self.client_id),
                    ("device_code", &code.device_code),
                    ("grant_type", DEVICE_GRANT),
                ],
            )?;
            match answer["error"].as_str() {
                None => return Grant::from_answer(&answer),
                Some("authorization_pending") => {}
                Some("slow_down") => {
                    interval = answer["interval"].as_u64().unwrap_or(interval + 5);
                }
                Some("expired_token") => break,
                Some("access_denied") => bail!("the login was cancelled on GitHub"),
                Some(error) => bail!("GitHub refused the login: {}", described(&answer, error)),
            }
            if now() >= deadline {
                break;
            }
        }
        bail!("the code expired before it was entered; run `hldr auth login` again")
    }

    fn renew(&self, refresh_token: &str) -> Result<Grant, RenewError> {
        let answer = self
            .form(
                "/login/oauth/access_token",
                &[
                    ("client_id", &self.client_id),
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh_token),
                ],
            )
            .map_err(RenewError::Unreachable)?;
        match answer["error"].as_str() {
            None => Grant::from_answer(&answer).map_err(RenewError::Unreachable),
            Some(error) => Err(RenewError::Refused(described(&answer, error))),
        }
    }

    /// Who a token belongs to, and when it expires as GitHub reports it for
    /// tokens that do.
    pub fn user(&self, token: &str) -> Result<(String, Option<u64>)> {
        let (status, expiry, body) = self.get(token, "/user")?;
        match status {
            200 => {}
            401 => bail!("GitHub rejected the token (401): it is wrong, expired or revoked"),
            code => bail!("GitHub answered {code} for /user: {}", first_line(&body)),
        }
        let user: Value = serde_json::from_str(&body).context("GitHub sent invalid JSON")?;
        let login = user["login"]
            .as_str()
            .context("GitHub named no user for the token")?
            .to_owned();
        Ok((login, expiry))
    }

    /// The repositories the app's installations grant this token, or `None`
    /// for a token that is not the app's.
    pub fn repositories(&self, token: &str) -> Result<Option<Vec<String>>> {
        let (status, _, body) = self.get(token, "/user/installations?per_page=100")?;
        if status == 403 {
            return Ok(None);
        }
        let installations = json_ok(status, &body, "/user/installations")?;
        let mut repositories = Vec::new();
        for id in installations["installations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|installation| installation["id"].as_u64())
        {
            let path = format!("/user/installations/{id}/repositories?per_page=100");
            let (status, _, body) = self.get(token, &path)?;
            let listed = json_ok(status, &body, &path)?;
            repositories.extend(
                listed["repositories"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|repository| repository["full_name"].as_str())
                    .map(str::to_owned),
            );
        }
        Ok(Some(repositories))
    }

    fn form(&self, path: &str, fields: &[(&str, &str)]) -> Result<Value> {
        let response = self
            .agent
            .post(format!("{}{path}", self.base))
            .header("Accept", "application/json")
            .send_form(fields.iter().copied())
            .context("cannot reach GitHub")?;
        let (status, body) = read(response)?;
        let answer: Value = serde_json::from_str(&body)
            .map_err(|_| anyhow!("GitHub answered {status} for {path}: {}", first_line(&body)))?;
        if !(200..300).contains(&status) && answer.get("error").is_none() {
            bail!("GitHub answered {status} for {path}");
        }
        Ok(answer)
    }

    fn get(&self, token: &str, path: &str) -> Result<(u16, Option<u64>, String)> {
        let response = self
            .agent
            .get(format!("{}{path}", self.api))
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call()
            .context("cannot reach GitHub")?;
        let expiry = response
            .headers()
            .get("github-authentication-token-expiration")
            .and_then(|value| value.to_str().ok())
            .and_then(github_time);
        let (status, body) = read(response)?;
        Ok((status, expiry, body))
    }
}

fn read(mut response: ureq::http::Response<ureq::Body>) -> Result<(u16, String)> {
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_string()
        .context("reading GitHub's answer")?;
    Ok((status, body))
}

fn json_ok(status: u16, body: &str, path: &str) -> Result<Value> {
    if !(200..300).contains(&status) {
        bail!("GitHub answered {status} for {path}: {}", first_line(body));
    }
    serde_json::from_str(body).with_context(|| format!("GitHub sent invalid JSON for {path}"))
}

fn described(answer: &Value, error: &str) -> String {
    match answer["error_description"].as_str() {
        Some(description) => format!("{description} ({error})"),
        None => error.to_owned(),
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// Runs the device flow and stores what it grants; the user it belongs to.
/// `show` tells the user the code and where to enter it.
pub fn login(
    store: &Store,
    oauth: &OAuth,
    show: &mut dyn FnMut(&DeviceCode) -> Result<()>,
    now: &dyn Fn() -> u64,
    sleep: &dyn Fn(Duration),
) -> Result<Credential> {
    let code = oauth.start()?;
    show(&code)?;
    let grant = oauth.wait(&code, now, sleep)?;
    let (user, _) = oauth.user(&grant.access_token)?;
    let credential = grant.credential(user, now());
    store.store(&credential)?;
    Ok(credential)
}

/// Checks a pasted token with GitHub and stores it.
pub fn login_with_token(store: &Store, oauth: &OAuth, token: &str) -> Result<Credential> {
    let token = token.trim();
    if token.is_empty() {
        bail!("no token given");
    }
    let (user, expires_at) = oauth.user(token)?;
    let credential = Credential::Token {
        user,
        token: token.to_owned(),
        expires_at,
    };
    store.store(&credential)?;
    Ok(credential)
}

/// The token for GitHub: `HLDR_GITHUB_TOKEN`, else the stored credential,
/// renewed first when it is about to expire; `None` when there is neither.
pub fn token(
    env_token: Option<&str>,
    store: Option<&Store>,
    oauth: &OAuth,
    term: &Term,
    now: u64,
) -> Result<Option<String>> {
    if let Some(token) = env_token {
        return Ok(Some(token.to_owned()));
    }
    let Some(store) = store else {
        return Ok(None);
    };
    match store.load()? {
        None => Ok(None),
        Some(Credential::Token {
            token, expires_at, ..
        }) => {
            if let Some(at) = expires_at {
                if at <= now {
                    bail!(
                        "the stored GitHub token expired; store a new one with `hldr auth login \
                         --with-token`, or run `hldr auth login`"
                    );
                }
                if at - now < TOKEN_WARNING {
                    term.warning(&format!(
                        "the stored GitHub token expires in {}; replace it with `hldr auth login`",
                        span(at - now)
                    ));
                }
            }
            Ok(Some(token))
        }
        Some(Credential::App {
            access_token,
            expires_at,
            ..
        }) if fresh(expires_at, now) => Ok(Some(access_token)),
        Some(Credential::App { .. }) => renew(store, oauth, term, now).map(Some),
    }
}

fn fresh(expires_at: Option<u64>, now: u64) -> bool {
    expires_at.is_none_or(|at| at > now + RENEW_AHEAD)
}

const SESSION_OVER: &str = "the GitHub session is over; run `hldr auth login`";

fn renew(store: &Store, oauth: &OAuth, term: &Term, now: u64) -> Result<String> {
    let _lock = store.lock()?;
    // Another run may have renewed while this one waited for the lock.
    let Some(Credential::App {
        user,
        access_token,
        expires_at,
        refresh_token,
        refresh_expires_at,
    }) = store.load()?
    else {
        bail!("the GitHub credential changed while renewing it; run the command again");
    };
    if fresh(expires_at, now) {
        return Ok(access_token);
    }
    let Some(refresh_token) =
        refresh_token.filter(|_| refresh_expires_at.is_none_or(|at| at > now))
    else {
        store.remove_locked()?;
        bail!("{SESSION_OVER}: it went unused for six months");
    };
    match oauth.renew(&refresh_token) {
        Ok(grant) => {
            let credential = grant.credential(user, now);
            store.save(&credential)?;
            match credential {
                Credential::App { access_token, .. } => Ok(access_token),
                Credential::Token { token, .. } => Ok(token),
            }
        }
        Err(RenewError::Refused(reason)) => {
            store.remove_locked()?;
            bail!("{SESSION_OVER}: GitHub refused to renew it, {reason}");
        }
        Err(RenewError::Unreachable(err)) => match expires_at {
            Some(at) if at > now + 60 => {
                term.warning(&format!(
                    "could not renew the GitHub session ({err:#}); the current token expires in {}",
                    span(at - now)
                ));
                Ok(access_token)
            }
            _ => Err(err.context("renewing the GitHub session")),
        },
    }
}

/// A duration as `183 days`, `7h41m` or `12m`.
pub fn span(secs: u64) -> String {
    match secs {
        s if s >= 2 * 86_400 => format!("{} days", s / 86_400),
        s if s >= 3600 => format!("{}h{:02}m", s / 3600, s % 3600 / 60),
        s => format!("{}m", s.div_ceil(60)),
    }
}

/// A token's expiry as GitHub reports it, such as `2027-09-22 12:00:00 UTC`
/// or `2027-09-22 12:00:00 -0300`, in seconds since the Unix epoch.
fn github_time(text: &str) -> Option<u64> {
    let mut parts = text.split_whitespace();
    let (date, time, zone) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let numbers = |text: &str, sep: char| -> Option<Vec<i64>> {
        text.split(sep).map(|part| part.parse().ok()).collect()
    };
    let (Some(&[year, month, day]), Some(&[hour, minute, second])) =
        (numbers(date, '-').as_deref(), numbers(time, ':').as_deref())
    else {
        return None;
    };
    let offset = match zone {
        "UTC" | "Z" => 0,
        _ => {
            let (sign, digits) = zone.split_at_checked(1)?;
            let sign = match sign {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            if digits.len() != 4 {
                return None;
            }
            let hours: i64 = digits[..2].parse().ok()?;
            let minutes: i64 = digits[2..].parse().ok()?;
            sign * (hours * 3600 + minutes * 60)
        }
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    u64::try_from(seconds - offset).ok()
}

/// Days since 1970-01-01 of a proleptic Gregorian date, after Howard
/// Hinnant's `days_from_civil`.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Why there is no [`Store`].
pub const NO_STORE: &str = "no place to keep a credential: neither XDG_STATE_HOME nor HOME is set";

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Form, Json, Router};
    use serde_json::json;

    use super::*;
    use crate::stub;

    const NOW: u64 = 1_800_000_000;

    /// A form as the token endpoint received it.
    type Sent = Vec<(String, String)>;

    /// GitHub's OAuth endpoints and `/user`, answering the token endpoint
    /// from a script and recording what it was sent.
    #[derive(Clone, Default)]
    struct GitHub {
        answers: Arc<Mutex<VecDeque<Value>>>,
        sent: Arc<Mutex<Vec<Sent>>>,
        /// What `/user` reports as the token's expiry.
        expiry: Option<&'static str>,
        /// How long the token endpoint takes to answer.
        delay: Duration,
    }

    impl GitHub {
        fn script(&self, answers: impl IntoIterator<Item = Value>) {
            self.answers.lock().unwrap().extend(answers);
        }

        fn sent(&self) -> Vec<Sent> {
            self.sent.lock().unwrap().clone()
        }

        fn serve(&self) -> OAuth {
            let app = Router::new()
                .route(
                    "/login/device/code",
                    post(|| async {
                        Json(json!({
                            "device_code": "dev", "user_code": "WDJB-MJHT",
                            "verification_uri": "https://github.com/login/device",
                            "expires_in": 900, "interval": 5,
                        }))
                    }),
                )
                .route("/login/oauth/access_token", post(token_endpoint))
                .route("/user", get(user))
                .route(
                    "/user/installations",
                    get(|| async { Json(json!({"installations": [{"id": 7}]})) }),
                )
                .route(
                    "/user/installations/7/repositories",
                    get(|| async { Json(json!({"repositories": [{"full_name": "o/content"}]})) }),
                )
                .with_state(self.clone());
            let base = stub::spawn(app);
            OAuth::new(&base, &base, "client")
        }
    }

    async fn token_endpoint(
        State(github): State<GitHub>,
        Form(form): Form<Vec<(String, String)>>,
    ) -> Json<Value> {
        github.sent.lock().unwrap().push(form);
        tokio::time::sleep(github.delay).await;
        let answer = github.answers.lock().unwrap().pop_front();
        Json(answer.unwrap_or_else(|| json!({"error": "unscripted"})))
    }

    async fn user(State(github): State<GitHub>, headers: HeaderMap) -> impl IntoResponse {
        if headers["authorization"] == "Bearer bad" {
            return (StatusCode::UNAUTHORIZED, HeaderMap::new(), "{}").into_response();
        }
        let mut answer = HeaderMap::new();
        if let Some(expiry) = github.expiry {
            answer.insert(
                "github-authentication-token-expiration",
                expiry.parse().unwrap(),
            );
        }
        (answer, Json(json!({"login": "hvpaiva"}))).into_response()
    }

    fn granted(access: &str, refresh: &str) -> Value {
        json!({
            "access_token": access, "expires_in": 28_800,
            "refresh_token": refresh, "refresh_token_expires_in": 15_897_600,
            "token_type": "bearer", "scope": "",
        })
    }

    fn app(expires_at: u64, refresh_expires_at: u64) -> Credential {
        Credential::App {
            user: "hvpaiva".to_owned(),
            access_token: "a1".to_owned(),
            expires_at: Some(expires_at),
            refresh_token: Some("r1".to_owned()),
            refresh_expires_at: Some(refresh_expires_at),
        }
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("state/hldr"));
        (dir, store)
    }

    /// An OAuth whose endpoints nobody answers.
    fn unreachable() -> OAuth {
        OAuth::new("http://127.0.0.1:1", "http://127.0.0.1:1", "client")
    }

    fn field<'a>(form: &'a [(String, String)], name: &str) -> Option<&'a str> {
        form.iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn the_device_flow_waits_at_githubs_pace() {
        let github = GitHub::default();
        github.script([
            json!({"error": "authorization_pending"}),
            json!({"error": "slow_down", "interval": 10}),
            granted("a1", "r1"),
        ]);
        let oauth = github.serve();
        let (_dir, store) = store();
        let slept = Mutex::new(Vec::new());
        let mut shown = None;
        let credential = login(
            &store,
            &oauth,
            &mut |code| {
                shown = Some(code.user_code.clone());
                Ok(())
            },
            &|| NOW,
            &|wait| slept.lock().unwrap().push(wait.as_secs()),
        )
        .unwrap();

        assert_eq!(shown.as_deref(), Some("WDJB-MJHT"));
        assert_eq!(*slept.lock().unwrap(), [5, 5, 10]);
        assert!(credential == app(NOW + 28_800, NOW + 15_897_600));
        assert!(store.load().unwrap() == Some(credential));
        let sent = github.sent();
        assert_eq!(field(&sent[0], "grant_type"), Some(DEVICE_GRANT));
        assert_eq!(field(&sent[0], "client_id"), Some("client"));
        assert_eq!(field(&sent[0], "client_secret"), None, "no secret, ever");
    }

    #[test]
    fn a_refused_or_expired_device_flow_says_so() {
        for (answer, expected) in [
            (json!({"error": "access_denied"}), "cancelled on GitHub"),
            (json!({"error": "expired_token"}), "code expired"),
            (
                json!({"error": "unsupported_grant_type", "error_description": "nope"}),
                "nope (unsupported_grant_type)",
            ),
        ] {
            let github = GitHub::default();
            github.script([answer]);
            let (_dir, store) = store();
            let err = login(&store, &github.serve(), &mut |_| Ok(()), &|| NOW, &|_| {})
                .err()
                .unwrap();
            assert!(err.to_string().contains(expected), "{err}");
            assert!(store.load().unwrap().is_none());
        }
    }

    #[test]
    fn a_fresh_token_needs_no_network() {
        let (_dir, store) = store();
        store.store(&app(NOW + 7200, NOW + 86_400)).unwrap();
        let token = token(None, Some(&store), &unreachable(), &Term::plain(), NOW).unwrap();
        assert_eq!(token.as_deref(), Some("a1"));
    }

    #[test]
    fn a_token_near_its_end_is_renewed_and_the_new_pair_kept() {
        let github = GitHub::default();
        github.script([granted("a2", "r2")]);
        let oauth = github.serve();
        let (_dir, store) = store();
        store.store(&app(NOW + 1800, NOW + 86_400)).unwrap();

        let token = token(None, Some(&store), &oauth, &Term::plain(), NOW).unwrap();
        assert_eq!(token.as_deref(), Some("a2"));
        let sent = github.sent();
        assert_eq!(field(&sent[0], "grant_type"), Some("refresh_token"));
        assert_eq!(field(&sent[0], "refresh_token"), Some("r1"));
        assert_eq!(field(&sent[0], "client_secret"), None);
        let Some(Credential::App {
            refresh_token,
            expires_at,
            ..
        }) = store.load().unwrap()
        else {
            panic!("the credential is gone");
        };
        assert_eq!(refresh_token.as_deref(), Some("r2"));
        assert_eq!(expires_at, Some(NOW + 28_800));
    }

    #[test]
    fn a_refused_renewal_ends_the_session() {
        let github = GitHub::default();
        github.script([json!({"error": "bad_refresh_token", "error_description": "revoked"})]);
        let (_dir, store) = store();
        store.store(&app(NOW + 1800, NOW + 86_400)).unwrap();
        let err = token(None, Some(&store), &github.serve(), &Term::plain(), NOW).unwrap_err();
        assert!(err.to_string().contains("run `hldr auth login`"), "{err}");
        assert!(err.to_string().contains("revoked"), "{err}");
        assert!(store.load().unwrap().is_none(), "the dead credential goes");
    }

    #[test]
    fn a_session_unused_for_six_months_is_over() {
        let (_dir, store) = store();
        store.store(&app(NOW - 10, NOW - 1)).unwrap();
        let err = token(None, Some(&store), &unreachable(), &Term::plain(), NOW).unwrap_err();
        assert!(err.to_string().contains("unused for six months"), "{err}");
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn github_out_of_reach_keeps_a_token_that_still_works() {
        let (_dir, store) = store();
        store.store(&app(NOW + 600, NOW + 86_400)).unwrap();
        let term = Term::plain();
        term.hold();
        let token = token(None, Some(&store), &unreachable(), &term, NOW).unwrap();
        assert_eq!(token.as_deref(), Some("a1"));
        let held = term.held();
        assert!(held[0].contains("could not renew"), "{held:?}");
        assert!(store.load().unwrap().is_some(), "nothing is dropped");

        store.store(&app(NOW + 30, NOW + 86_400)).unwrap();
        let err = super::token(None, Some(&store), &unreachable(), &term, NOW).unwrap_err();
        assert!(
            format!("{err:#}").contains("renewing the GitHub session"),
            "{err:#}"
        );
    }

    #[test]
    fn concurrent_runs_renew_once() {
        let github = GitHub {
            delay: Duration::from_millis(200),
            ..GitHub::default()
        };
        github.script([granted("a2", "r2"), granted("a3", "r3")]);
        let oauth = Arc::new(github.serve());
        let (_dir, store) = store();
        store.store(&app(NOW + 1800, NOW + 86_400)).unwrap();
        let store = Arc::new(store);
        let runs: Vec<_> = (0..2)
            .map(|_| {
                let (oauth, store) = (Arc::clone(&oauth), Arc::clone(&store));
                std::thread::spawn(move || {
                    token(None, Some(&store), &oauth, &Term::plain(), NOW).unwrap()
                })
            })
            .collect();
        for run in runs {
            assert_eq!(run.join().unwrap().as_deref(), Some("a2"));
        }
        assert_eq!(github.sent().len(), 1, "the refresh token is spent once");
    }

    #[test]
    fn the_environment_wins_and_nothing_stored_is_none() {
        let (_dir, store) = store();
        let term = Term::plain();
        assert_eq!(
            token(None, Some(&store), &unreachable(), &term, NOW).unwrap(),
            None
        );
        store.store(&app(NOW - 10, NOW - 1)).unwrap();
        assert_eq!(
            token(Some("env"), Some(&store), &unreachable(), &term, NOW)
                .unwrap()
                .as_deref(),
            Some("env")
        );
        assert_eq!(token(None, None, &unreachable(), &term, NOW).unwrap(), None);
    }

    #[test]
    fn a_pasted_token_keeps_the_expiry_github_reports() {
        let github = GitHub {
            expiry: Some("2027-09-22 12:00:00 UTC"),
            ..GitHub::default()
        };
        let oauth = github.serve();
        let (_dir, store) = store();
        let credential = login_with_token(&store, &oauth, "  pat\n").unwrap();
        assert!(
            credential
                == Credential::Token {
                    user: "hvpaiva".to_owned(),
                    token: "pat".to_owned(),
                    expires_at: Some(1_821_614_400),
                }
        );
        assert!(login_with_token(&store, &oauth, "bad").is_err());
        assert!(login_with_token(&store, &oauth, " ").is_err());

        let term = Term::plain();
        term.hold();
        let near = 1_821_614_400 - 3 * 86_400;
        let token = token(None, Some(&store), &unreachable(), &term, near).unwrap();
        assert_eq!(token.as_deref(), Some("pat"));
        assert!(
            term.held()[0].contains("expires in 3 days"),
            "{:?}",
            term.held()
        );
        let err =
            super::token(None, Some(&store), &unreachable(), &term, 1_821_614_400).unwrap_err();
        assert!(err.to_string().contains("expired"), "{err}");
    }

    #[test]
    fn lists_what_the_app_reaches() {
        let oauth = GitHub::default().serve();
        assert_eq!(
            oauth.repositories("a1").unwrap(),
            Some(vec!["o/content".to_owned()])
        );
    }

    #[test]
    fn the_file_is_private_and_refused_otherwise() {
        let (_dir, store) = store();
        store.store(&app(NOW, NOW)).unwrap();
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&store.path()), 0o600);
        assert_eq!(mode(&store.dir), 0o700);

        std::fs::set_permissions(store.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = store.load().err().unwrap();
        assert!(err.to_string().contains("chmod 600"), "{err}");
        assert!(store.remove().unwrap(), "a refused file can still go");
        assert!(!store.remove().unwrap());
    }

    #[test]
    fn another_format_is_refused() {
        let (_dir, store) = store();
        store.store(&app(NOW, NOW)).unwrap();
        std::fs::write(store.path(), r#"{"version": 2}"#).unwrap();
        let err = store.load().err().unwrap();
        assert!(err.to_string().contains("format 2"), "{err}");
    }

    #[test]
    fn reads_githubs_expiry_times() {
        assert_eq!(github_time("2027-09-22 12:00:00 UTC"), Some(1_821_614_400));
        assert_eq!(
            github_time("2027-09-22 12:00:00 -0300"),
            Some(1_821_625_200)
        );
        assert_eq!(
            github_time("2027-09-22 15:00:00 +0300"),
            Some(1_821_614_400)
        );
        assert_eq!(github_time("1970-01-01 00:00:00 UTC"), Some(0));
        for bad in [
            "",
            "2027-09-22",
            "2027-13-01 00:00:00 UTC",
            "2027-09-22 12:00 UTC",
            "x y z",
        ] {
            assert_eq!(github_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn spans_read_at_a_glance() {
        assert_eq!(span(30), "1m");
        assert_eq!(span(12 * 60), "12m");
        assert_eq!(span(7 * 3600 + 41 * 60), "7h41m");
        assert_eq!(span(183 * 86_400 + 5), "183 days");
    }
}
