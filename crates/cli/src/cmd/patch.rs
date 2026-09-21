use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use hldr_core::manifest::{self, Manifest};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::cmd::Writer;
use crate::content::{self, Publish};
use crate::github::Change;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Resource type, such as `project` or `site`
    #[arg(value_name = "TYPE")]
    kind: String,
    /// Resource name; singletons take none
    name: Option<String>,
    /// JSON merge patch (RFC 7396) over `{"spec": ...}`, such as
    /// '{"spec":{"tagline":"New line"}}'; null removes an optional field.
    /// The file is rewritten from its fields, so YAML comments and layout in
    /// its frontmatter are not kept.
    #[arg(short, long, value_name = "JSON")]
    patch: String,
    /// Show the result without committing
    #[arg(long)]
    dry_run: bool,
    #[command(flatten)]
    write: super::WriteArgs,
}

pub fn run(out: &mut dyn Write, writer: &mut Writer<'_>, args: &Args) -> Result<bool> {
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
    let current = manifest::parse(Path::new(&path), original.as_bytes())?;
    let patched = patch_manifest(current, &patch)?;
    let rendered = manifest::render(&patched)?;
    content::validate(&path, &rendered)?;

    let label = match &args.name {
        Some(name) => format!("{}/{name}", resource.singular),
        None => resource.singular.clone(),
    };
    if rendered == original {
        writeln!(out, "{label} patched (no change)")?;
        return Ok(true);
    }
    if args.dry_run {
        write!(
            out,
            "{}",
            super::diff::unified(
                &original,
                &rendered,
                &format!("a/{path}"),
                &format!("b/{path}")
            )
        )?;
        return Ok(true);
    }
    writeln!(out, "{label} patched")?;
    Publish {
        client: writer.client,
        target: &writer.target,
        parent: &head,
        message: args.write.message(&format!("patch {label}")),
        sync: args.write.sync(),
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

/// Applies a merge patch to a manifest's spec, refusing fields the kind
/// does not have instead of dropping them.
pub fn patch_manifest(current: Manifest, patch: &Value) -> Result<Manifest> {
    let Value::Object(document) = patch else {
        bail!("the patch must be a JSON object such as {{\"spec\": {{...}}}}");
    };
    if let Some(key) = document.keys().find(|key| *key != "spec") {
        bail!(
            "only spec can be patched; {key:?} is kept by the server or fixed by the file's place"
        );
    }
    let Some(spec_patch) = document.get("spec") else {
        bail!("the patch changes nothing: it has no spec");
    };
    Ok(match current {
        Manifest::Project { name, spec } => Manifest::Project {
            name,
            spec: merge_spec(&spec, spec_patch)?,
        },
        Manifest::Theme { name, spec } => Manifest::Theme {
            name,
            spec: merge_spec(&spec, spec_patch)?,
        },
        Manifest::Profile(spec) => Manifest::Profile(merge_spec(&spec, spec_patch)?),
        Manifest::Site(spec) => Manifest::Site(merge_spec(&spec, spec_patch)?),
    })
}

fn merge_spec<T: Serialize + DeserializeOwned>(spec: &T, patch: &Value) -> Result<T> {
    let mut merged = serde_json::to_value(spec)?;
    merge(&mut merged, patch);
    let typed: T =
        serde_json::from_value(merged.clone()).context("the patched spec is not valid")?;
    let round_trip = serde_json::to_value(&typed)?;
    if let Some(field) = unknown(&merged, &round_trip, "spec") {
        bail!("{field} is not a field of this kind; `hldr explain` lists the fields");
    }
    Ok(typed)
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

/// The first path present in `merged` that did not survive typing.
fn unknown(merged: &Value, typed: &Value, at: &str) -> Option<String> {
    let (Value::Object(merged), Value::Object(typed)) = (merged, typed) else {
        return None;
    };
    merged.iter().find_map(|(key, value)| match typed.get(key) {
        None if !value.is_null() => Some(format!("{at}.{key}")),
        None => None,
        Some(kept) => unknown(value, kept, &format!("{at}.{key}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROJECT: &str = "---\nkind: Project\ntitle: Atlas\ntagline: Old\nstatus: active\ntags: [rust]\n---\n\nBody.\n";

    fn project() -> Manifest {
        manifest::parse(Path::new("projects/atlas.md"), PROJECT.as_bytes()).unwrap()
    }

    fn spec(manifest: &Manifest) -> Value {
        match manifest {
            Manifest::Project { spec, .. } => serde_json::to_value(spec).unwrap(),
            _ => unreachable!(),
        }
    }

    #[test]
    fn merges_into_the_spec() {
        let patched = patch_manifest(
            project(),
            &json!({"spec": {"tagline": "New", "draft": true, "links": {"repo": "https://x.test"}}}),
        )
        .unwrap();
        let spec = spec(&patched);
        assert_eq!(spec["tagline"], "New");
        assert_eq!(spec["draft"], true);
        assert_eq!(spec["links"]["repo"], "https://x.test");
        assert_eq!(spec["body"], "\nBody.\n");
        let rendered = manifest::render(&patched).unwrap();
        assert!(rendered.ends_with("---\n\nBody.\n"), "{rendered}");
    }

    #[test]
    fn null_removes_an_optional_field() {
        let with = patch_manifest(project(), &json!({"spec": {"highlight": 2}})).unwrap();
        let without = patch_manifest(with, &json!({"spec": {"highlight": null}})).unwrap();
        assert!(spec(&without)["highlight"].is_null());
    }

    #[test]
    fn refuses_what_it_cannot_patch() {
        let cases = [
            (json!({"spec": {"tagln": "x"}}), "spec.tagln is not a field"),
            (json!({"spec": {"links": {"rep": "x"}}}), "not valid"),
            (
                json!({"metadata": {"name": "x"}}),
                "only spec can be patched",
            ),
            (json!({"spec": {"status": "done"}}), "not valid"),
            (json!({"spec": {"title": null}}), "not valid"),
            (json!([1]), "must be a JSON object"),
            (json!({}), "has no spec"),
        ];
        for (patch, expected) in cases {
            let err = patch_manifest(project(), &patch).unwrap_err();
            assert!(format!("{err:#}").contains(expected), "{patch}: {err:#}");
        }
    }
}
