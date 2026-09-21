//! The private API over HTTP.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use hldr_core::api::Problem;
use serde_json::Value;

const TIMEOUT: Duration = Duration::from_secs(60);
const MAX_BODY: u64 = 16 * 1024 * 1024;

pub struct Client {
    agent: ureq::Agent,
    base: String,
}

impl Client {
    pub fn new(base: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .user_agent(format!("hldr/{}", hldr_core::VERSION))
            .build()
            .into();
        Self { agent, base }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// A resource that must exist.
    pub fn get(&self, path: &str) -> Result<Value> {
        self.get_optional(path)?
            .ok_or_else(|| anyhow!("{path}: not found on {}", self.base))
    }

    /// A resource that may be missing: a 404 is `None`, any other failure an
    /// error carrying the server's explanation.
    pub fn get_optional(&self, path: &str) -> Result<Option<Value>> {
        let request = self.agent.get(format!("{}{path}", self.base));
        self.answer(path, request.call())
    }

    fn answer(
        &self,
        path: &str,
        response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<Option<Value>> {
        let mut response = response.with_context(|| {
            format!(
                "cannot reach {} (the API is only served to the tailnet)",
                self.base
            )
        })?;
        let status = response.status();
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_string()
            .with_context(|| format!("{path}: reading the response"))?;
        if status.is_success() {
            return serde_json::from_str(&body)
                .map(Some)
                .with_context(|| format!("{path}: the server sent invalid JSON"));
        }
        if status == 404 {
            return Ok(None);
        }
        Err(match serde_json::from_str::<Problem>(&body) {
            Ok(problem) => anyhow!("{} ({})", problem.detail, status.as_u16()),
            Err(_) => anyhow!(
                "{path}: {} {}",
                status.as_u16(),
                body.lines().next().unwrap_or_default()
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stub::Stub;

    #[test]
    fn maps_answers_to_results() {
        let client = Client::new(Stub::default().serve());
        assert_eq!(
            client.get("/api/v1/projects/atlas").unwrap()["kind"],
            "Project"
        );
        assert!(
            client
                .get_optional("/api/v1/projects/nope")
                .unwrap()
                .is_none()
        );

        let problem = client.get("/api/v1/broken").unwrap_err();
        assert_eq!(problem.to_string(), "content is invalid (422)");
        let plain = client.get("/api/v1/plain").unwrap_err();
        assert_eq!(plain.to_string(), "/api/v1/plain: 500 internal error");
    }

    #[test]
    fn explains_an_unreachable_server() {
        let client = Client::new("http://127.0.0.1:9".to_owned());
        let err = client.get("/healthz").unwrap_err();
        assert!(
            err.to_string().contains("only served to the tailnet"),
            "{err}"
        );
    }
}
