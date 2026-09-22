use std::io::Write;
use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::cmd::Writer;
use crate::color::{Role, Term};
use crate::content::{self, Publish};
use crate::github::Change;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Manifest, content directory, or `-` for a singleton on stdin. A
    /// page's markdown comes along from beside its manifest.
    #[arg(short = 'f', long = "filename", value_name = "PATH", required = true)]
    files: Vec<PathBuf>,
    /// Show what would change without committing
    #[arg(long)]
    dry_run: bool,
    #[command(flatten)]
    write: super::WriteArgs,
}

/// Makes the branch hold these files as they are, in one commit: each
/// manifest with the markdown it names. Files it already holds unchanged
/// are left out; nothing absent from the arguments is removed.
pub fn run(out: &mut dyn Write, term: &Term, writer: &mut Writer<'_>, args: &Args) -> Result<bool> {
    if !args.dry_run {
        writer.authorize()?;
    }
    let registry = writer.registry()?;
    let files = content::collect(&mut writer.catalog, &registry, &args.files, "apply")?;
    let head = writer.target.head()?;
    let mut changes = Vec::new();
    let mut labels = Vec::new();
    let suffix = if args.dry_run { " (dry run)" } else { "" };
    for file in &files {
        let current = writer.target.github.file(&head, &file.path)?;
        let mut state = match &current {
            None => "created",
            Some(text) if *text == file.text => "unchanged",
            Some(_) => "configured",
        };
        if state != "unchanged" {
            changes.push(Change {
                path: file.path.clone(),
                content: Some(file.text.clone()),
            });
        }
        match (&file.content, file.content_path()) {
            (Some(markdown), _) => {
                let current = writer.target.github.file(&head, &markdown.path)?;
                if current.as_deref() != Some(markdown.text.as_str()) {
                    if state == "unchanged" {
                        state = "configured";
                    }
                    changes.push(Change {
                        path: markdown.path.clone(),
                        content: Some(markdown.text.clone()),
                    });
                }
            }
            (None, Some(path)) => {
                if writer.target.github.file(&head, &path)?.is_none() {
                    bail!(
                        "{}: names {path} as its content, which is neither beside it nor on {}",
                        file.origin,
                        writer.target.branch
                    );
                }
            }
            (None, None) => {}
        }
        let paint = term.out();
        let key = match state {
            "created" => Role::ApplyCreated,
            "configured" => Role::ApplyConfigured,
            _ => Role::ApplyUnchanged,
        };
        writeln!(
            out,
            "{} {}{}",
            file.label(),
            paint.paint(key, state),
            paint.paint(Role::ApplyDryRun, suffix)
        )?;
        if state != "unchanged" {
            labels.push(file.label());
        }
    }
    if args.dry_run || changes.is_empty() {
        return Ok(true);
    }
    let resources = writer.resources()?;
    Publish {
        client: writer.client,
        target: &writer.target,
        resources: &resources,
        parent: &head,
        message: args.write.message(&format!("apply {}", labels.join(", "))),
        sync: args.write.sync(),
    }
    .run(out, &changes)?;
    Ok(true)
}
