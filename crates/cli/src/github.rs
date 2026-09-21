//! The content repository on GitHub: reading files at a commit, and writing
//! a set of changes as one commit.
//!
//! A write builds a tree on top of the commit the CLI read, commits it, and
//! moves the branch only if it still points at that commit (`force: false`).
//! Anything pushed in between makes the write fail instead of being
//! overwritten: optimistic concurrency at the branch.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

pub const API: &str = "https://api.github.com";
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
        let mut response = self
            .authorized(request)
            .call()
            .with_context(|| format!("cannot reach GitHub for {path}"))?;
        let status = response.status();
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_string()
            .with_context(|| format!("reading {path} from GitHub"))?;
        match status.as_u16() {
            200 => Ok(Some(body)),
            404 => Ok(None),
            code => Err(github_error(code, &body)).with_context(|| format!("reading {path}")),
        }
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
            bail!(
                "writing needs a GitHub token: set HLDR_GITHUB_TOKEN, or `content.token_command` \
                 in the config file"
            );
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
        let mut response = response.with_context(|| format!("cannot reach GitHub for {path}"))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_string()
            .with_context(|| format!("reading GitHub's answer to {method} {path}"))?;
        Ok((status, text))
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
        401 => ": the token is invalid or expired",
        403 | 404 => ": the token may lack Contents write access to this repository",
        _ => "",
    };
    anyhow!("GitHub answered {message} ({status}){hint}")
}
