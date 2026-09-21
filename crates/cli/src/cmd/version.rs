use std::io::Write;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::client::Client;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Only the client's version; no server needed.
    #[arg(long)]
    client: bool,
    /// Plain text, or json or yaml
    #[arg(short, long)]
    output: Option<String>,
}

pub fn run(out: &mut dyn Write, client: Option<&Client>, args: &Args) -> Result<bool> {
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
        if let Some(warning) = skew(hldr_core::VERSION, health["version"].as_str()) {
            eprintln!("warning: {warning}");
        }
    }
    match args.output.as_deref() {
        None => write!(out, "{}", text(&report))?,
        Some("json") => writeln!(out, "{}", serde_json::to_string_pretty(&report)?)?,
        Some("yaml") => write!(out, "{}", serde_saphyr::to_string(&report)?)?,
        Some(other) => bail!("unknown output {other:?}: use json or yaml"),
    }
    Ok(true)
}

impl Args {
    pub fn client_only(&self) -> bool {
        self.client
    }
}

/// Client and server ship from the same tag, so any difference means one of
/// them is behind: the client may parse content in a way the server does not.
fn skew(client: &str, server: Option<&str>) -> Option<String> {
    let server = server.unwrap_or("unknown");
    (client != server).then(|| {
        format!(
            "client {client} and server {server} differ; install the hldr released with the \
             server (see `gh release download` in the hldr README)"
        )
    })
}

fn text(report: &Value) -> String {
    let field = |value: &Value| value.as_str().unwrap_or("<none>").to_owned();
    let mut out = format!(
        "Client Version: {} ({})\n",
        field(&report["client"]["version"]),
        field(&report["client"]["revision"])
    );
    if report["server"].is_object() {
        out.push_str(&format!(
            "Server Version: {} ({}) at {}\n",
            field(&report["server"]["version"]),
            field(&report["server"]["revision"]),
            field(&report["server"]["url"])
        ));
        let content = &report["content"];
        let revision = content["revision"]
            .as_str()
            .map_or_else(|| "<none>".to_owned(), |r| r.chars().take(12).collect());
        out.push_str(&format!(
            "Content:        {revision} from {}, synced {}\n",
            field(&content["source"]),
            field(&content["synced_at"])
        ));
        if let Some(error) = content["last_error"].as_str() {
            let first = error.lines().next().unwrap_or(error);
            out.push_str(&format!("Last sync failed: {first}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_when_client_and_server_differ() {
        assert_eq!(skew("3.5.0", Some("3.5.0")), None);
        let warning = skew("3.4.0", Some("3.5.0")).unwrap();
        assert!(
            warning.starts_with("client 3.4.0 and server 3.5.0 differ"),
            "{warning}"
        );
        assert!(skew("dev", None).unwrap().contains("server unknown"));
    }

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
            text(&report),
            "Client Version: dev (unknown)\n\
             Server Version: 3.2.0 (abc) at http://x\n\
             Content:        20592fd6ffe2 from github.com/o/r@main, synced 2026-09-21T19:21:25Z\n\
             Last sync failed: index: site.yaml: bad\n"
        );
    }
}
