use std::io::Write;

use anyhow::{Context, Result, bail};
use hldr_core::manifest::Manifest;
use serde_json::{Map, Value};

use crate::cmd::Writer;
use crate::color::{Role, Term};
use crate::content::{self, Publish};
use crate::github::Change;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Resource type, such as `project` or `site`
    #[arg(value_name = "TYPE")]
    kind: String,
    /// Resource name; singletons take none
    name: Option<String>,
    /// JSON merge patch (RFC 7396) over the manifest: `spec`, and a page's
    /// `metadata`, such as '{"metadata":{"tagline":"New line"}}'; null
    /// removes an optional field. The manifest is rewritten from its fields,
    /// so YAML comments and layout are not kept.
    #[arg(short, long, value_name = "JSON")]
    patch: String,
    /// Show the result without committing
    #[arg(long)]
    dry_run: bool,
    #[command(flatten)]
    write: super::WriteArgs,
}

pub fn run(out: &mut dyn Write, term: &Term, writer: &mut Writer<'_>, args: &Args) -> Result<bool> {
    let paint = term.out();
    let patch: Value = serde_json::from_str(&args.patch).context("--patch is not JSON")?;
    if !args.dry_run {
        writer.authorize()?;
    }
    let resource = writer.resource(&args.kind, "patch")?;
    let path = content::path_for(&resource, args.name.as_deref())?;
    let head = writer.target.head()?;
    let Some(original) = writer.target.github.file(&head, &path)? else {
        bail!("{path} does not exist on {}", writer.target.branch);
    };
    let registry = writer.registry()?;
    let current = content::validate(&registry, &path, &original)?;
    let document: Value =
        serde_saphyr::from_str(&original).with_context(|| format!("reading {path}"))?;
    let patched = patch_document(
        document.clone(),
        &patch,
        matches!(current, Manifest::Page(_)),
    )?;

    let label = match &args.name {
        Some(name) => format!("{}/{name}", resource.singular),
        None => resource.singular.clone(),
    };
    if patched == document {
        writeln!(
            out,
            "{label} {}",
            paint.paint(Role::ApplyUnchanged, "patched (no change)")
        )?;
        return Ok(true);
    }
    let rendered = serde_saphyr::to_string(&patched)?;
    content::validate(&registry, &path, &rendered)?;
    if args.dry_run {
        let diff = super::diff::unified(
            &original,
            &rendered,
            &format!("a/{path}"),
            &format!("b/{path}"),
        );
        write!(out, "{}", super::diff::colored(paint, &diff))?;
        return Ok(true);
    }
    writeln!(
        out,
        "{label} {}",
        paint.paint(Role::PatchPatched, "patched")
    )?;
    let resources = writer.resources()?;
    Publish {
        client: writer.client,
        target: &writer.target,
        resources: &resources,
        parent: &head,
        message: args.write.message(&format!("patch {label}")),
        sync: args.write.sync(),
        paint,
    }
    .run(
        out,
        &[Change {
            path,
            content: Some(rendered),
        }],
    )?;
    Ok(true)
}

/// Applies a merge patch to a manifest's `spec`, and to a page's
/// `metadata`, leaving what the file's place fixes alone: its kind and its
/// name. The parser then refuses fields the kind does not have.
pub fn patch_document(mut document: Value, patch: &Value, page: bool) -> Result<Value> {
    let Value::Object(parts) = patch else {
        bail!("the patch must be a JSON object such as {{\"spec\": {{...}}}}");
    };
    if parts.is_empty() {
        bail!("the patch changes nothing: it has no spec");
    }
    for (part, value) in parts {
        match part.as_str() {
            "spec" => {}
            "metadata" if page => {
                let Value::Object(fields) = value else {
                    bail!("metadata must be an object of fields");
                };
                for key in fields.keys() {
                    match key.as_str() {
                        "name" => {
                            bail!("metadata.name is the file's name; rename the file instead")
                        }
                        "created_at" | "updated_at" => {
                            bail!("metadata.{key} is kept by the server")
                        }
                        _ => {}
                    }
                }
            }
            "metadata" => bail!(
                "only a page's metadata can be patched; this kind's metadata is its name, fixed by the file's place"
            ),
            other => bail!(
                "only spec{} can be patched; {other:?} is kept by the server or fixed by the file's place",
                if page { " and metadata" } else { "" }
            ),
        }
        merge(&mut document[part.as_str()], value);
    }
    Ok(document)
}

/// RFC 7396: objects merge key by key, null removes, anything else replaces.
fn merge(target: &mut Value, patch: &Value) {
    let Value::Object(patch) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    if let Value::Object(target) = target {
        for (key, value) in patch {
            if value.is_null() {
                target.remove(key);
            } else {
                merge(target.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn atlas() -> Value {
        json!({
            "kind": "Project",
            "metadata": {"name": "atlas", "title": "Atlas", "tagline": "Old", "highlight": 1},
            "spec": {"content": {"file": "atlas.md"}},
        })
    }

    #[test]
    fn merges_metadata_and_spec() {
        let patched = patch_document(
            atlas(),
            &json!({"metadata": {"tagline": "New", "links": {"repo": "https://x.test"}}, "spec": {"draft": true}}),
            true,
        )
        .unwrap();
        assert_eq!(patched["metadata"]["tagline"], "New");
        assert_eq!(patched["metadata"]["links"]["repo"], "https://x.test");
        assert_eq!(patched["metadata"]["name"], "atlas");
        assert_eq!(patched["spec"]["draft"], true);
        assert_eq!(patched["spec"]["content"]["file"], "atlas.md");
    }

    #[test]
    fn null_removes_an_optional_field() {
        let patched =
            patch_document(atlas(), &json!({"metadata": {"highlight": null}}), true).unwrap();
        assert!(patched["metadata"].get("highlight").is_none());
    }

    #[test]
    fn refuses_what_it_cannot_patch() {
        let cases = [
            (json!({"metadata": {"name": "x"}}), true, "rename the file"),
            (
                json!({"metadata": {"updated_at": "x"}}),
                true,
                "kept by the server",
            ),
            (
                json!({"metadata": {"title": "x"}}),
                false,
                "only a page's metadata",
            ),
            (
                json!({"kind": "Theme"}),
                true,
                "only spec and metadata can be patched",
            ),
            (json!({"status": {}}), false, "only spec can be patched"),
            (json!([1]), true, "must be a JSON object"),
            (json!({}), true, "changes nothing"),
        ];
        for (patch, page, expected) in cases {
            let err = patch_document(atlas(), &patch, page).unwrap_err();
            assert!(format!("{err:#}").contains(expected), "{patch}: {err:#}");
        }
    }
}
