use std::io::Write;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::client::Client;
use crate::color::{self, Painter, Role, Term};
use crate::skew;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Only the client's version; no server needed.
    #[arg(long)]
    client: bool,
    /// Plain text, or json or yaml
    #[arg(short, long)]
    output: Option<String>,
}

pub fn run(out: &mut dyn Write, term: &Term, client: Option<&Client>, args: &Args) -> Result<bool> {
    let mut report = json!({
        "client": {"version": hldr_core::VERSION, "revision": hldr_core::REVISION},
    });
    if let Some(client) = client {
        let health = client.get("/healthz")?;
        let sync = client.get("/api/v1/sync")?;
        report["server"] = json!({
            "url": client.base(),
            "version": health["version"],
            "revision": health["revision"],
        });
        report["content"] = json!({
            "source": sync["source"],
            "revision": sync["revision"],
            "synced_at": sync["synced_at"],
            "last_error": sync["last_error"],
        });
        let server = health["version"].as_str().unwrap_or("unknown");
        if let Some(warning) = skew::explain(hldr_core::VERSION, server) {
            term.warning(&warning);
        }
    }
    let paint = term.out();
    match args.output.as_deref() {
        None => write!(out, "{}", text(paint, &report))?,
        Some("json") => {
            let text = serde_json::to_string_pretty(&report)?;
            writeln!(out, "{}", color::json(paint, &text))?;
        }
        Some("yaml") => write!(
            out,
            "{}",
            color::yaml(paint, &serde_saphyr::to_string(&report)?)
        )?,
        Some(other) => bail!("unknown output {other:?}: use json or yaml"),
    }
    Ok(true)
}

impl Args {
    pub fn client_only(&self) -> bool {
        self.client
    }
}

fn text(paint: Painter<'_>, report: &Value) -> String {
    let field = |value: &Value| paint.value(value.as_str().unwrap_or("<none>"));
    let key = |name: &str| paint.nth(Role::VersionKey, 0, name);
    let mut out = format!(
        "{}: {} ({})\n",
        key("Client Version"),
        field(&report["client"]["version"]),
        field(&report["client"]["revision"])
    );
    if report["server"].is_object() {
        out.push_str(&format!(
            "{}: {} ({}) at {}\n",
            key("Server Version"),
            field(&report["server"]["version"]),
            field(&report["server"]["revision"]),
            field(&report["server"]["url"])
        ));
        let content = &report["content"];
        let revision = content["revision"]
            .as_str()
            .map_or_else(|| "<none>".to_owned(), |r| r.chars().take(12).collect());
        out.push_str(&format!(
            "{}:        {} from {}, synced {}\n",
            key("Content"),
            paint.value(&revision),
            field(&content["source"]),
            field(&content["synced_at"])
        ));
        if let Some(error) = content["last_error"].as_str() {
            let first = error.lines().next().unwrap_or(error);
            out.push_str(&format!(
                "{}: {}\n",
                key("Last sync failed"),
                paint.paint(Role::StatusError, first)
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prints_client_server_and_content() {
        let report = json!({
            "client": {"version": "dev", "revision": "unknown"},
            "server": {"url": "http://x", "version": "3.2.0", "revision": "abc"},
            "content": {
                "source": "github.com/o/r@main",
                "revision": "20592fd6ffe2bf85510d434b9ac537a26f3d9dfd",
                "synced_at": "2026-09-21T19:21:25Z",
                "last_error": "index: site.yaml: bad\ndetail",
            },
        });
        assert_eq!(
            text(Term::plain().out(), &report),
            "Client Version: dev (unknown)\n\
             Server Version: 3.2.0 (abc) at http://x\n\
             Content:        20592fd6ffe2 from github.com/o/r@main, synced 2026-09-21T19:21:25Z\n\
             Last sync failed: index: site.yaml: bad\n"
        );
    }
}
