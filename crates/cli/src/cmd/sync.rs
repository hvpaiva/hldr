use std::io::Write;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::client::Client;
use crate::content::{self, Target};

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Show what the server serves and whether the branch is ahead of it
    Status {
        /// Plain text, or json or yaml
        #[arg(short, long)]
        output: Option<String>,
    },
}

/// `hldr sync` has the server fetch its branch now instead of on its next
/// poll; `hldr sync status` compares what it serves with the branch.
pub fn run(
    out: &mut dyn Write,
    client: &Client,
    api: &str,
    token: &dyn Fn() -> Result<Option<String>>,
    args: &Args,
) -> Result<bool> {
    match &args.command {
        None => {
            let status = client.post("/api/v1/sync", &json!({}))?;
            writeln!(out, "synced: {}", content::summary(&status))?;
            Ok(true)
        }
        Some(Command::Status { output }) => {
            let mut status = client.get("/api/v1/sync")?;
            let head = if status["repository"].is_null() {
                None
            } else {
                Some(Target::discover(client, api, token()?)?.head()?)
            };
            status["branch_head"] = head.clone().map_or(Value::Null, Value::String);
            match output.as_deref() {
                None => write!(out, "{}", text(&status))?,
                Some("json") => writeln!(out, "{}", serde_json::to_string_pretty(&status)?)?,
                Some("yaml") => write!(out, "{}", serde_saphyr::to_string(&status)?)?,
                Some(other) => bail!("unknown output {other:?}: use json or yaml"),
            }
            Ok(true)
        }
    }
}

fn text(status: &Value) -> String {
    let field = |key: &str| status[key].as_str().unwrap_or("<none>").to_owned();
    let served = status["revision"].as_str();
    let head = status["branch_head"].as_str();
    let state = match (served, head) {
        (Some(served), Some(head)) if served == head => "Synced".to_owned(),
        (_, Some(head)) => format!(
            "OutOfSync: {} is at {}; `hldr sync` brings the site there",
            field("branch"),
            content::short(head)
        ),
        (_, None) => "Unknown: content comes from a directory".to_owned(),
    };
    let mut out = format!(
        "Source:      {}\nServed:      {} (synced {})\nState:       {state}\nLast Try:    {}\n",
        field("source"),
        served.map_or("<none>", content::short),
        field("synced_at"),
        field("last_attempt_at"),
    );
    if let Some(error) = status["last_error"].as_str() {
        out.push_str("Last Error:\n");
        for line in error.lines() {
            out.push_str(&format!("    {line}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_follow_the_branch() {
        let mut status = json!({
            "source": "github.com/o/r@main", "branch": "main",
            "revision": "a".repeat(40), "synced_at": "t1", "last_attempt_at": "t2",
            "last_error": null, "branch_head": "a".repeat(40),
        });
        assert!(
            text(&status).contains("State:       Synced\n"),
            "{}",
            text(&status)
        );
        status["branch_head"] = json!("b".repeat(40));
        assert!(
            text(&status).contains("OutOfSync: main is at bbbbbbbbbbbb"),
            "{}",
            text(&status)
        );
        status["last_error"] = json!("index: site.yaml: bad\n  detail");
        assert!(text(&status).ends_with("Last Error:\n    index: site.yaml: bad\n      detail\n"));
    }
}
