//! The content repository on GitHub: reading files at a commit, and writing
//! a set of changes as one commit.
//!
//! A write builds a tree on top of the commit the CLI read, commits it, and
//! moves the branch only if it still points at that commit (`force: false`).
//! Anything pushed in between makes the write fail instead of being
//! overwritten: optimistic concurrency at the branch.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use hldr_core::github::{self, RateLimit};
use serde::Serialize;
use serde_json::{Value, json};

pub const API: &str = "https://api.github.com";

/// What a write without GitHub access says.
pub const NOT_LOGGED_IN: &str =
    "writing needs GitHub access: run `hldr auth login`, or set HLDR_GITHUB_TOKEN";
const TIMEOUT: Duration = Duration::from_secs(60);
const MAX_BODY: u64 = 16 * 1024 * 1024;

/// A file to write, or to remove when `content` is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub content: Option<String>,
}

pub struct GitHub {
    agent: ureq::Agent,
    base: String,
    repo: String,
    token: Option<String>,
}

impl GitHub {
    /// `repo` is `owner/name`, as the server reports it.
    pub fn new(api: &str, repo: &str, token: Option<String>) -> Result<Self> {
        let valid = repo.split_once('/').is_some_and(|(owner, name)| {
            [owner, name].iter().all(|part| {
                !part.is_empty()
                    && !part.starts_with('.')
                    && part
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            })
        });
        if !valid {
            bail!("the server reports an invalid repository {repo:?}");
        }
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .user_agent(format!("hldr/{}", hldr_core::VERSION))
            .build()
            .into();
        Ok(Self {
            agent,
            base: format!("{}/repos/{repo}", api.trim_end_matches('/')),
            repo: repo.to_owned(),
            token,
        })
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }

    /// The commit a branch points at. The query parameter keeps an anonymous
    /// read from being answered by GitHub's cache.
    pub fn head(&self, branch: &str) -> Result<String> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let path = format!("/git/ref/heads/{branch}?fresh={nonce}");
        let reference = self.call("GET", &path, None)?;
        sha(&reference["object"]["sha"])
            .with_context(|| format!("branch {branch} of {}", self.repo))
    }

    /// A file at a commit, or `None` when it does not exist there.
    pub fn file(&self, commit: &str, path: &str) -> Result<Option<String>> {
        let request = self
            .agent
            .get(format!("{}/contents/{path}?ref={commit}", self.base))
            .header("Accept", "application/vnd.github.raw");
        let response = self
            .authorized(request)
            .call()
            .with_context(|| format!("cannot reach GitHub for {path}"))?;
        let (status, body) = answer(response).with_context(|| format!("reading {path}"))?;
        match status {
            200 => Ok(Some(body)),
            404 => Ok(None),
            code => Err(github_error(code, &body)).with_context(|| format!("reading {path}")),
        }
    }

    /// The latest commits of `branch`, newest first, or only those that
    /// changed `path`. Asked fresh, as [`GitHub::head`] is, so a commit a
    /// write just made shows.
    pub fn commits(&self, branch: &str, path: Option<&str>, limit: u16) -> Result<Vec<Commit>> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let mut query = format!("/commits?sha={branch}&per_page={limit}&fresh={nonce}");
        if let Some(path) = path {
            if !is_content_path(path) {
                bail!(
                    "{path:?} is not a path in the content repository, such as projects/atlas.yaml"
                );
            }
            query.push_str(&format!("&path={path}"));
        }
        let list = self.call("GET", &query, None)?;
        list.as_array()
            .context("GitHub sent no list of commits")?
            .iter()
            .map(Commit::from_github)
            .collect()
    }

    /// One commit, and the files it changed.
    pub fn commit_files(&self, revision: &str) -> Result<(Commit, Vec<FileChange>)> {
        let valid =
            (4..=40).contains(&revision.len()) && revision.chars().all(|c| c.is_ascii_hexdigit());
        if !valid {
            bail!("{revision:?} is not a commit id: 4 to 40 hex digits");
        }
        let path = format!("/commits/{revision}");
        let (status, body) = self.request("GET", &path, None)?;
        let detail: Value = match status {
            200 => serde_json::from_str(&body)
                .with_context(|| format!("GitHub sent invalid JSON for {path}"))?,
            404 | 422 => bail!("{} has no commit {revision}", self.repo),
            code => return Err(github_error(code, &body)).with_context(|| format!("GET {path}")),
        };
        let files = detail["files"]
            .as_array()
            .map(|files| files.iter().map(FileChange::from_github).collect())
            .transpose()?
            .unwrap_or_default();
        Ok((Commit::from_github(&detail)?, files))
    }

    /// Commits `changes` on top of `parent` and moves `branch` to the new
    /// commit, unless the branch moved since `parent` was read.
    pub fn commit(
        &self,
        branch: &str,
        parent: &str,
        changes: &[Change],
        message: &str,
    ) -> Result<String> {
        if self.token.is_none() {
            bail!("{NOT_LOGGED_IN}");
        }
        let base_tree =
            sha(&self.call("GET", &format!("/git/commits/{parent}"), None)?["tree"]["sha"])?;
        let entries: Vec<Value> = changes
            .iter()
            .map(|change| match &change.content {
                Some(content) => json!({"path": change.path, "mode": "100644", "type": "blob", "content": content}),
                None => json!({"path": change.path, "mode": "100644", "type": "blob", "sha": null}),
            })
            .collect();
        let tree = sha(&self.call(
            "POST",
            "/git/trees",
            Some(json!({"base_tree": base_tree, "tree": entries})),
        )?["sha"])?;
        let commit = sha(&self.call(
            "POST",
            "/git/commits",
            Some(json!({"message": message, "tree": tree, "parents": [parent]})),
        )?["sha"])?;
        let (status, body) = self.request(
            "PATCH",
            &format!("/git/refs/heads/{branch}"),
            Some(json!({"sha": commit, "force": false})),
        )?;
        match status {
            200..=299 => Ok(commit),
            422 => bail!(
                "{branch} moved while this change was being written; nothing was changed. \
                 Run the command again to start from the new {branch}"
            ),
            code => Err(github_error(code, &body)).with_context(|| format!("moving {branch}")),
        }
    }

    /// A request whose answer must be a success with a JSON body.
    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let (status, text) = self.request(method, path, body)?;
        if !(200..300).contains(&status) {
            return Err(github_error(status, &text)).with_context(|| format!("{method} {path}"));
        }
        serde_json::from_str(&text).with_context(|| format!("GitHub sent invalid JSON for {path}"))
    }

    fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<(u16, String)> {
        let url = format!("{}{path}", self.base);
        let response = match (method, body) {
            ("GET", None) => self.authorized(self.agent.get(&url)).call(),
            ("POST", Some(body)) => self.send(self.agent.post(&url), &body)?,
            ("PATCH", Some(body)) => self.send(self.agent.patch(&url), &body)?,
            _ => bail!("unsupported request {method} {path}"),
        };
        let response = response.with_context(|| format!("cannot reach GitHub for {path}"))?;
        answer(response).with_context(|| format!("{method} {path}"))
    }

    fn send(
        &self,
        request: ureq::RequestBuilder<ureq::typestate::WithBody>,
        body: &Value,
    ) -> Result<Result<ureq::http::Response<ureq::Body>, ureq::Error>> {
        let json = serde_json::to_string(body)?;
        Ok(self
            .authorized(request)
            .header("Accept", "application/vnd.github+json")
            .header("Content-Type", "application/json")
            .send(json))
    }

    fn authorized<B>(&self, request: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        let request = request.header("X-GitHub-Api-Version", "2022-11-28");
        match &self.token {
            Some(token) => request.header("Authorization", format!("Bearer {token}")),
            None => request,
        }
    }
}

/// An answer's status and body; a spent rate limit is an error that says
/// when it resets, whatever the request was.
fn answer(mut response: ureq::http::Response<ureq::Body>) -> Result<(u16, String)> {
    let status = response.status().as_u16();
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    let now = github::now();
    if let Some(limit) = RateLimit::from_answer(
        status,
        header("x-ratelimit-remaining"),
        header("x-ratelimit-reset"),
        header("retry-after"),
        now,
    ) {
        bail!("{}", limit.message(now));
    }
    let text = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_string()
        .context("reading GitHub's answer")?;
    Ok((status, text))
}

/// A commit of the content repository, as `hldr history` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Commit {
    pub revision: String,
    pub author: String,
    /// When it was authored, ISO-8601 UTC.
    pub date: String,
    pub message: String,
}

impl Commit {
    fn from_github(value: &Value) -> Result<Self> {
        let commit = &value["commit"];
        let text = |field: &Value| field.as_str().unwrap_or_default().to_owned();
        Ok(Self {
            revision: sha(&value["sha"])?,
            author: text(&commit["author"]["name"]),
            date: text(&commit["author"]["date"]),
            message: text(&commit["message"]),
        })
    }

    /// The first line of the message.
    pub fn subject(&self) -> &str {
        self.message.lines().next().unwrap_or_default()
    }
}

/// A file a commit changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileChange {
    pub path: String,
    /// `added`, `modified`, `removed` or `renamed`, as GitHub words it.
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    /// Where a renamed file was before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_path: Option<String>,
}

impl FileChange {
    fn from_github(value: &Value) -> Result<Self> {
        Ok(Self {
            path: value["filename"]
                .as_str()
                .context("GitHub sent a changed file without a name")?
                .to_owned(),
            status: value["status"].as_str().unwrap_or("modified").to_owned(),
            additions: value["additions"].as_u64().unwrap_or(0),
            deletions: value["deletions"].as_u64().unwrap_or(0),
            previous_path: value["previous_filename"].as_str().map(str::to_owned),
        })
    }
}

/// A relative path of plain segments, which goes into a query string as it
/// is: content paths need no escaping, and anything else is refused.
fn is_content_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
}

fn sha(value: &Value) -> Result<String> {
    value
        .as_str()
        .filter(|sha| sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("GitHub sent no commit id where one was expected"))
}

/// GitHub's own message, which names the missing permission or the conflict.
fn github_error(status: u16, body: &str) -> anyhow::Error {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.lines().next().unwrap_or_default().to_owned());
    let hint = match status {
        401 => ": the credential is wrong, expired or revoked; run `hldr auth login`".to_owned(),
        403 | 404 => format!(
            ": the credential may not reach this repository with Contents write access; for the \
             hldr app, install it on the repository at {}",
            crate::auth::INSTALLATIONS
        ),
        _ => String::new(),
    };
    anyhow!("GitHub answered {message} ({status}){hint}")
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::get;

    use super::*;

    #[test]
    fn a_spent_rate_limit_says_when_it_resets() {
        let reset = github::now() + 600;
        let app = Router::new().route(
            "/repos/o/r/git/ref/heads/main",
            get(move || async move {
                (
                    StatusCode::FORBIDDEN,
                    [
                        ("x-ratelimit-remaining", "0".to_owned()),
                        ("x-ratelimit-reset", reset.to_string()),
                    ],
                    r#"{"message":"API rate limit exceeded for 203.0.113.9."}"#,
                )
            }),
        );
        let hub = GitHub::new(&crate::stub::spawn(app), "o/r", None).unwrap();
        let err = format!("{:#}", hub.head("main").unwrap_err());
        assert!(
            err.contains("GitHub rate limit exceeded; it resets at "),
            "{err}"
        );
        assert!(err.contains("in 10 min"), "{err}");
    }
}
