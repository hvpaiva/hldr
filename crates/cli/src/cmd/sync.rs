use std::io::Write;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::client::Client;
use crate::color::{self, Painter, Role, Term};
use crate::content::{self, Target};
use crate::discovery::Catalog;

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
    term: &Term,
    client: &Client,
    catalog: &mut Catalog<'_>,
    api: &str,
    token: &dyn Fn() -> Result<Option<String>>,
    args: &Args,
) -> Result<bool> {
    match &args.command {
        None => {
            let status = client.post("/api/v1/sync", &json!({}))?;
            let resources = &catalog.discovery()?.resources;
            writeln!(out, "{}", content::synced(term.out(), &status, resources))?;
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
            let paint = term.out();
            match output.as_deref() {
                None => write!(out, "{}", text(paint, &status))?,
                Some("json") => {
                    let text = serde_json::to_string_pretty(&status)?;
                    writeln!(out, "{}", color::json(paint, &text))?;
                }
                Some("yaml") => write!(
                    out,
                    "{}",
                    color::yaml(paint, &serde_saphyr::to_string(&status)?)
                )?,
                Some(other) => bail!("unknown output {other:?}: use json or yaml"),
            }
            Ok(true)
        }
    }
}

/// Laid out as `describe` lays out a resource, the state colored as a
/// status is.
fn text(paint: Painter<'_>, status: &Value) -> String {
    let raw = |key: &str| status[key].as_str().unwrap_or("<none>").to_owned();
    let field = |key: &str| paint.value(&raw(key));
    let label = |name: &str| paint.nth(Role::DescribeKey, 0, name);
    let served = status["revision"].as_str();
    let head = status["branch_head"].as_str();
    let state = match (served, head) {
        (Some(served), Some(head)) if served == head => paint.paint(Role::StatusSuccess, "Synced"),
        (_, Some(head)) => format!(
            "{}: {} is at {}; `hldr sync` brings the site there",
            paint.paint(Role::StatusWarning, "OutOfSync"),
            raw("branch"),
            content::short(head)
        ),
        (_, None) => format!(
            "{}: content comes from a directory",
            paint.paint(Role::StatusWarning, "Unknown")
        ),
    };
    let mut out = format!(
        "{}:      {}\n{}:      {} (synced {})\n{}:       {state}\n{}:    {}\n",
        label("Source"),
        field("source"),
        label("Served"),
        paint.value(served.map_or("<none>", content::short)),
        field("synced_at"),
        label("State"),
        label("Last Try"),
        field("last_attempt_at"),
    );
    if let Some(error) = status["last_error"].as_str() {
        out.push_str(&format!("{}:\n", label("Last Error")));
        for line in error.lines() {
            out.push_str(&format!("    {}\n", paint.paint(Role::StatusError, line)));
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
            text(Term::plain().out(), &status).contains("State:       Synced\n"),
            "{}",
            text(Term::plain().out(), &status)
        );
        status["branch_head"] = json!("b".repeat(40));
        assert!(
            text(Term::plain().out(), &status).contains("OutOfSync: main is at bbbbbbbbbbbb"),
            "{}",
            text(Term::plain().out(), &status)
        );
        status["last_error"] = json!("index: site.yaml: bad\n  detail");
        assert!(
            text(Term::plain().out(), &status)
                .ends_with("Last Error:\n    index: site.yaml: bad\n      detail\n")
        );
    }
}
