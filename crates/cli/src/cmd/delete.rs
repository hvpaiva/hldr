use std::io::Write;

use anyhow::{Result, bail};

use crate::cmd::Writer;
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

/// Removes the resources' files in one commit; git keeps them in history.
/// A missing name is reported and the others are still deleted.
pub fn run(out: &mut dyn Write, writer: &mut Writer<'_>, args: &Args) -> Result<bool> {
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
        if writer.target.github.file(&head, &path)?.is_none() {
            eprintln!("error: {} {name:?} not found", resource.singular);
            complete = false;
            continue;
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
    Publish {
        client: writer.client,
        target: &writer.target,
        parent: &head,
        message: args.write.message(&format!("delete {}", labels.join(", "))),
        sync: args.write.sync(),
    }
    .run(out, &changes)?;
    for label in &labels {
        writeln!(out, "{label} deleted")?;
    }
    Ok(complete)
}
