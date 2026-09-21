use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;

use crate::cmd::Writer;
use crate::content::{self, Publish};
use crate::github::Change;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Content file, content directory, or `-` for a singleton on stdin
    #[arg(short = 'f', long = "filename", value_name = "PATH", required = true)]
    files: Vec<PathBuf>,
    /// Show what would change without committing
    #[arg(long)]
    dry_run: bool,
    #[command(flatten)]
    write: super::WriteArgs,
}

/// Makes the branch hold these files as they are, in one commit. Files it
/// already holds unchanged are left out; nothing absent from the arguments
/// is removed.
pub fn run(out: &mut dyn Write, writer: &mut Writer<'_>, args: &Args) -> Result<bool> {
    if !args.dry_run {
        writer.authorize()?;
    }
    let files = content::collect(&mut writer.catalog, &args.files, "apply")?;
    let head = writer.target.head()?;
    let mut changes = Vec::new();
    let mut labels = Vec::new();
    let suffix = if args.dry_run { " (dry run)" } else { "" };
    for file in &files {
        let current = writer.target.github.file(&head, &file.path)?;
        let state = match &current {
            None => "created",
            Some(text) if *text == file.text => "unchanged",
            Some(_) => "configured",
        };
        writeln!(out, "{} {state}{suffix}", file.label())?;
        if state != "unchanged" {
            labels.push(file.label());
            changes.push(Change {
                path: file.path.clone(),
                content: Some(file.text.clone()),
            });
        }
    }
    if args.dry_run || changes.is_empty() {
        return Ok(true);
    }
    Publish {
        client: writer.client,
        target: &writer.target,
        parent: &head,
        message: args.write.message(&format!("apply {}", labels.join(", "))),
        sync: args.write.sync(),
    }
    .run(out, &changes)?;
    Ok(true)
}
