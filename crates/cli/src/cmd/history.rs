//! `hldr history`: the commits of the content repository, as `kubectl
//! rollout history` lists revisions. Read from GitHub; the server only says
//! which commit it serves.

use std::io::Write;

use anyhow::{Result, bail};
use hldr_core::api::Column;
use serde_json::{Value, json};

use crate::client::Client;
use crate::color::{self, Painter, Role, Term};
use crate::content::{self, Target};
use crate::github::{Commit, FileChange};
use crate::print;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Only the commits that changed this file, such as projects/atlas.yaml
    #[arg(value_name = "PATH")]
    path: Option<String>,
    /// Show one commit and the files it changed
    #[arg(short, long, value_name = "SHA", conflicts_with = "path")]
    revision: Option<String>,
    /// How many commits to list, newest first
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=100))]
    limit: u16,
    /// A table, or json or yaml
    #[arg(short, long)]
    output: Option<String>,
}

pub fn run(
    out: &mut dyn Write,
    term: &Term,
    client: &Client,
    api: &str,
    token: &dyn Fn() -> Result<Option<String>>,
    args: &Args,
) -> Result<bool> {
    let format = match args.output.as_deref() {
        None => None,
        Some(format @ ("json" | "yaml")) => Some(format),
        Some(other) => bail!("unknown output {other:?}: use json or yaml"),
    };
    let target = Target::discover(client, api, token()?)?;
    let paint = term.out();
    let served = target.served.as_deref();
    let value = match &args.revision {
        Some(revision) => {
            let (commit, files) = target.github.commit_files(revision)?;
            if format.is_none() {
                write!(out, "{}", commit_text(paint, &commit, &files, served)?)?;
                return Ok(true);
            }
            let mut value = serde_json::to_value(&commit)?;
            value["kind"] = json!("Commit");
            value["served"] = json!(served == Some(commit.revision.as_str()));
            value["files"] = serde_json::to_value(&files)?;
            value
        }
        None => {
            let commits =
                target
                    .github
                    .commits(&target.branch, args.path.as_deref(), args.limit)?;
            if format.is_none() {
                if commits.is_empty() {
                    term.warning("no commit changed that path");
                    return Ok(true);
                }
                history_table(out, paint, &commits, served)?;
                return Ok(true);
            }
            let items: Vec<Value> = commits
                .iter()
                .map(|commit| {
                    let mut item = serde_json::to_value(commit)?;
                    item["served"] = json!(served == Some(commit.revision.as_str()));
                    Ok(item)
                })
                .collect::<Result<_>>()?;
            json!({
                "kind": "History",
                "repository": target.github.repo(),
                "branch": target.branch,
                "served": served,
                "items": items,
            })
        }
    };
    match format {
        Some("yaml") => write!(
            out,
            "{}",
            color::yaml(paint, &serde_saphyr::to_string(&value)?)
        )?,
        _ => writeln!(
            out,
            "{}",
            color::json(paint, &serde_json::to_string_pretty(&value)?)
        )?,
    }
    Ok(true)
}

/// A status column: `served` drawn as good, as a synced state is.
fn served_column() -> Column {
    Column {
        name: "SERVED".to_owned(),
        values: vec!["served".to_owned()],
        ok: vec!["served".to_owned()],
        ..Column::default()
    }
}

fn history_table(
    out: &mut dyn Write,
    paint: Painter<'_>,
    commits: &[Commit],
    served: Option<&str>,
) -> std::io::Result<()> {
    let rows = commits
        .iter()
        .map(|commit| {
            let mark = if served == Some(commit.revision.as_str()) {
                "served"
            } else {
                ""
            };
            vec![
                content::short(&commit.revision).to_owned(),
                commit.date.clone(),
                commit.author.clone(),
                mark.to_owned(),
                commit.subject().to_owned(),
            ]
        })
        .collect();
    let headers = ["REVISION", "DATE", "AUTHOR", "SERVED", "SUBJECT"]
        .map(str::to_owned)
        .to_vec();
    let plain = Column::default();
    let served = served_column();
    print::table(
        out,
        paint,
        headers,
        rows,
        &[&plain, &plain, &plain, &served],
        false,
    )
}

/// One commit laid out as `describe` lays out a resource, then its files.
fn commit_text(
    paint: Painter<'_>,
    commit: &Commit,
    files: &[FileChange],
    served: Option<&str>,
) -> Result<String> {
    let label = |name: &str| paint.nth(Role::DescribeKey, 0, name);
    let is_served = served == Some(commit.revision.as_str());
    let mut out = format!(
        "{}:   {}\n{}:     {}\n{}:       {}\n{}:     {}\n{}:\n",
        label("Revision"),
        paint.paint(Role::DataString, &commit.revision),
        label("Author"),
        paint.paint(Role::DataString, &commit.author),
        label("Date"),
        paint.paint(Role::DataString, &commit.date),
        label("Served"),
        paint.value(if is_served { "true" } else { "false" }),
        label("Message"),
    );
    for line in commit.message.trim_end().lines() {
        if line.is_empty() {
            out.push('\n');
        } else {
            out.push_str(&format!("    {}\n", paint.paint(Role::DataString, line)));
        }
    }
    if files.is_empty() {
        out.push_str(&format!(
            "{}:     {}\n",
            label("Files"),
            paint.paint(Role::DataNull, "<none>")
        ));
        return Ok(out);
    }
    out.push_str(&format!("{}:\n", label("Files")));
    let rows = files
        .iter()
        .map(|file| {
            let path = match &file.previous_path {
                Some(before) => format!("{before} → {}", file.path),
                None => file.path.clone(),
            };
            vec![
                file.status.clone(),
                format!("+{}", file.additions),
                format!("-{}", file.deletions),
                path,
            ]
        })
        .collect();
    let status = Column {
        name: "STATUS".to_owned(),
        values: ["added", "modified", "renamed", "removed"]
            .map(str::to_owned)
            .to_vec(),
        ok: ["added", "modified", "renamed"].map(str::to_owned).to_vec(),
        ..Column::default()
    };
    let mut table = Vec::new();
    print::table(
        &mut table,
        paint,
        ["STATUS", "+", "-", "PATH"].map(str::to_owned).to_vec(),
        rows,
        &[&status],
        false,
    )?;
    for line in String::from_utf8_lossy(&table).lines() {
        out.push_str(&format!("  {line}\n"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::stub::hub::{self, Repo};

    fn run_history(repo: &hub::Shared, argv: &[&str]) -> Result<String> {
        use clap::Parser;
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: Args,
        }
        let mut full = vec!["history"];
        full.extend(argv);
        let args = Cli::try_parse_from(full)?.args;
        let base = hub::serve(Arc::clone(repo));
        let client = Client::new(base.clone());
        let mut out = Vec::new();
        run(
            &mut out,
            &Term::plain(),
            &client,
            &base,
            &|| Ok(None),
            &args,
        )?;
        Ok(String::from_utf8(out)?)
    }

    fn repo() -> (hub::Shared, Vec<String>) {
        let mut repo = Repo::default();
        let first = repo.push(&[("site.yaml", "a\n")], "content: create site");
        let second = repo.push(
            &[("projects/atlas.yaml", "one\ntwo\n")],
            "content: add atlas\n\nWith a body.",
        );
        let third = repo.push(&[("site.yaml", "b\n")], "content: retitle site");
        repo.served = Some(second.clone());
        (Arc::new(Mutex::new(repo)), vec![first, second, third])
    }

    #[test]
    fn lists_commits_newest_first_and_marks_the_served_one() {
        let (repo, ids) = repo();
        let out = run_history(&repo, &[]).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 4, "{out}");
        assert!(lines[0].starts_with("REVISION       DATE"), "{out}");
        assert!(lines[1].starts_with(content::short(&ids[2])), "{out}");
        assert!(lines[1].ends_with("content: retitle site"), "{out}");
        assert!(lines[2].contains("   served   content: add atlas"), "{out}");
        assert!(!lines[3].contains("served"), "{out}");
    }

    #[test]
    fn a_path_lists_only_the_commits_that_changed_it() {
        let (repo, _) = repo();
        let out = run_history(&repo, &["site.yaml"]).unwrap();
        assert_eq!(out.lines().count(), 3, "{out}");
        assert!(!out.contains("atlas"), "{out}");
        let out = run_history(&repo, &["--limit", "1"]).unwrap();
        assert_eq!(out.lines().count(), 2, "{out}");
        let err = run_history(&repo, &["../etc/passwd"]).unwrap_err();
        assert!(err.to_string().contains("not a path"), "{err}");
    }

    #[test]
    fn a_revision_shows_its_message_and_files() {
        let (repo, ids) = repo();
        let out = run_history(&repo, &["--revision", &ids[1]]).unwrap();
        assert_eq!(
            out,
            format!(
                "\
Revision:   {}
Author:     Hub Author
Date:       2026-09-22T12:00:04Z
Served:     true
Message:
    content: add atlas

    With a body.
Files:
  STATUS   +    -    PATH
  added    +2   -0   projects/atlas.yaml
",
                ids[1]
            )
        );
        let err = run_history(&repo, &["-r", "zzz"]).unwrap_err();
        assert!(err.to_string().contains("not a commit id"), "{err}");
        let err = run_history(&repo, &["-r", &"f".repeat(40)]).unwrap_err();
        assert!(err.to_string().contains("has no commit"), "{err}");
    }

    #[test]
    fn json_carries_what_the_table_shows() {
        let (repo, ids) = repo();
        let out = run_history(&repo, &["-o", "json"]).unwrap();
        let history: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(history["kind"], "History");
        assert_eq!(history["repository"], "o/r");
        assert_eq!(history["served"], ids[1].as_str());
        assert_eq!(history["items"][1]["served"], true);
        assert_eq!(history["items"][0]["message"], "content: retitle site");

        let out = run_history(&repo, &["-r", &ids[2], "-o", "yaml"]).unwrap();
        assert!(out.contains("kind: Commit"), "{out}");
        assert!(out.contains("status: modified"), "{out}");
        assert!(run_history(&repo, &["-o", "wide"]).is_err());
    }
}
