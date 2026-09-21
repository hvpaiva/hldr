use std::io::Write;

use anyhow::{Result, bail};

use crate::cmd::Writer;
use crate::content::{self, Publish};
use crate::editor;
use crate::github::Change;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Resource type, such as `project` or `site`
    #[arg(value_name = "TYPE")]
    kind: String,
    /// Resource name; singletons take none
    name: Option<String>,
    #[command(flatten)]
    write: super::WriteArgs,
}

/// Opens the resource's file as the branch has it, and commits what the
/// editor saves once it validates.
pub fn run(out: &mut dyn Write, writer: &mut Writer<'_>, args: &Args) -> Result<bool> {
    writer.authorize()?;
    let resource = writer.resource(&args.kind, "edit")?;
    let path = content::path_for(&resource, args.name.as_deref())?;
    let head = writer.target.head()?;
    let Some(original) = writer.target.github.file(&head, &path)? else {
        bail!(
            "{path} does not exist on {}; create it with `hldr apply -f`",
            writer.target.branch
        );
    };
    let editor = editor::command(writer.editor.as_deref());
    let edited = editor::edit(&editor, &editor::file_name(&path), &original, |text| {
        content::validate(&path, text)
    })?;
    let Some(edited) = edited else {
        return Ok(true);
    };
    let label = match &args.name {
        Some(name) => format!("{}/{name}", resource.singular),
        None => resource.singular.clone(),
    };
    Publish {
        client: writer.client,
        target: &writer.target,
        parent: &head,
        message: args.write.message(&format!("edit {label}")),
        sync: args.write.sync(),
    }
    .run(
        out,
        &[Change {
            path,
            content: Some(edited),
        }],
    )?;
    Ok(true)
}
