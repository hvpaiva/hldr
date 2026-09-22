use std::io::Write;

use anyhow::{Result, bail};

use crate::cmd::Writer;
use crate::color::Term;
use crate::content::{self, Publish};
use crate::editor;
use crate::github::Change;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Resource type, such as `page` or `site`
    #[arg(value_name = "TYPE")]
    kind: String,
    /// Resource name; singletons take none
    name: Option<String>,
    /// Edit the page's markdown, the file its manifest names, instead of the
    /// manifest
    #[arg(long)]
    content: bool,
    #[command(flatten)]
    write: super::WriteArgs,
}

/// Opens the resource's manifest, or its markdown, as the branch has it,
/// and commits what the editor saves once it validates.
pub fn run(out: &mut dyn Write, term: &Term, writer: &mut Writer<'_>, args: &Args) -> Result<bool> {
    writer.authorize()?;
    let resource = writer.resource(&args.kind, "edit")?;
    let manifest_path = content::path_for(&resource, args.name.as_deref())?;
    let head = writer.target.head()?;
    let Some(manifest) = writer.target.github.file(&head, &manifest_path)? else {
        bail!(
            "{manifest_path} does not exist on {}; create it with `hldr apply -f`",
            writer.target.branch
        );
    };
    let registry = writer.registry()?;
    let (path, original) = if args.content {
        let Some(path) = content::named_content(&manifest_path, &manifest) else {
            bail!(
                "{manifest_path} names no content file: its markdown is inline, so edit the manifest"
            );
        };
        let Some(original) = writer.target.github.file(&head, &path)? else {
            bail!(
                "{path}, which {manifest_path} names, does not exist on {}",
                writer.target.branch
            );
        };
        (path, original)
    } else {
        (manifest_path.clone(), manifest.clone())
    };
    let editor = editor::command(writer.editor.as_deref());
    let edited = editor::edit(&editor, &editor::file_name(&path), &original, |text| {
        if args.content {
            let page = content::validate(&registry, &manifest_path, &manifest)?;
            content::validate_content(&registry, &page, text)
        } else {
            content::validate(&registry, &path, text).map(|_| ())
        }
    })?;
    let Some(edited) = edited else {
        return Ok(true);
    };
    let label = match &args.name {
        Some(name) => format!("{}/{name}", resource.singular),
        None => resource.singular.clone(),
    };
    let resources = writer.resources()?;
    Publish {
        client: writer.client,
        target: &writer.target,
        resources: &resources,
        parent: &head,
        message: args.write.message(&format!("edit {label}")),
        sync: args.write.sync(),
        paint: term.out(),
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
