use std::io::Write;

use anyhow::{Result, bail};

use crate::cmd::Writer;
use crate::color::{Role, Term};
use crate::content::{self, Publish};
use crate::github::Change;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Resource type, such as `project`
    #[arg(value_name = "TYPE")]
    kind: String,
    /// Names to delete, in one commit
    #[arg(required = true)]
    names: Vec<String>,
    #[command(flatten)]
    write: super::WriteArgs,
}

/// Removes the resources' files in one commit, a page's markdown with its
/// manifest; git keeps them in history. A missing name is reported and the
/// others are still deleted.
pub fn run(out: &mut dyn Write, term: &Term, writer: &mut Writer<'_>, args: &Args) -> Result<bool> {
    writer.authorize()?;
    let resource = writer.resource(&args.kind, "delete")?;
    if resource.singleton {
        bail!("{} is a singleton and cannot be deleted", resource.singular);
    }
    let head = writer.target.head()?;
    let mut complete = true;
    let mut changes = Vec::new();
    let mut labels = Vec::new();
    for name in &args.names {
        let path = content::path_for(&resource, Some(name))?;
        let Some(text) = writer.target.github.file(&head, &path)? else {
            term.error(&format!("{} {name:?} not found", resource.singular));
            complete = false;
            continue;
        };
        if let Some(markdown) = content::named_content(&path, &text)
            && writer.target.github.file(&head, &markdown)?.is_some()
        {
            changes.push(Change {
                path: markdown,
                content: None,
            });
        }
        changes.push(Change {
            path,
            content: None,
        });
        labels.push(format!("{}/{name}", resource.singular));
    }
    if changes.is_empty() {
        return Ok(false);
    }
    let resources = writer.resources()?;
    Publish {
        client: writer.client,
        target: &writer.target,
        resources: &resources,
        parent: &head,
        message: args.write.message(&format!("delete {}", labels.join(", "))),
        sync: args.write.sync(),
    }
    .run(out, &changes)?;
    for label in &labels {
        writeln!(
            out,
            "{label} {}",
            term.out().paint(Role::DeleteDeleted, "deleted")
        )?;
    }
    Ok(complete)
}
