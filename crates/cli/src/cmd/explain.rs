//! Field documentation from the JSON Schema the server publishes, which
//! carries the doc comments of the wire types.

use std::io::Write;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use crate::discovery::Catalog;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// TYPE or TYPE.FIELD.PATH, such as `project.spec.links`
    #[arg(value_name = "TYPE[.FIELD]")]
    target: String,
}

pub fn run(out: &mut dyn Write, catalog: &mut Catalog<'_>, args: &Args) -> Result<bool> {
    let mut parts = args.target.split('.');
    let kind_name = parts.next().unwrap_or_default();
    let resource = catalog.resolve(kind_name)?;
    let schema = catalog
        .discovery()?
        .schemas
        .get(&resource.kind)
        .with_context(|| format!("the server publishes no schema for {}", resource.kind))?
        .clone();
    let empty = Map::new();
    let defs = schema["$defs"].as_object().unwrap_or(&empty);

    let mut node = &schema;
    let mut path: Vec<&str> = Vec::new();
    for part in parts {
        let parent = resolve(node, defs);
        let Some(child) = parent["properties"].get(part) else {
            let at = if path.is_empty() {
                resource.kind.clone()
            } else {
                format!("{}.{}", resource.kind, path.join("."))
            };
            bail!("field {part:?} does not exist in {at}");
        };
        path.push(part);
        node = child;
    }
    write!(out, "{}", explain(&resource.kind, &path, node, defs))?;
    Ok(true)
}

pub fn explain(kind: &str, path: &[&str], node: &Value, defs: &Map<String, Value>) -> String {
    let resolved = resolve(node, defs);
    let mut out = format!("KIND:     {kind}\n");
    if !path.is_empty() {
        out.push_str(&format!(
            "FIELD:    {} <{}>\n",
            path.join("."),
            type_name(node, defs)
        ));
    }
    out.push_str("\nDESCRIPTION:\n");
    let description = node["description"]
        .as_str()
        .or(resolved["description"].as_str());
    out.push_str(&indented(description.unwrap_or("<empty>"), 4));

    if let Some(values) = enum_values(resolved) {
        out.push_str("\nVALUES:\n");
        for (value, doc) in values {
            out.push_str(&format!("    {value}\n"));
            if let Some(doc) = doc {
                out.push_str(&indented(doc, 8));
            }
        }
    }

    if let Some(properties) = resolved["properties"].as_object() {
        let required: Vec<&str> = resolved["required"]
            .as_array()
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        out.push_str("\nFIELDS:\n");
        for (name, field) in properties {
            let flag = if required.contains(&name.as_str()) {
                " -required-"
            } else {
                ""
            };
            out.push_str(&format!("  {name}\t<{}>{flag}\n", type_name(field, defs)));
            let doc = field["description"]
                .as_str()
                .or(resolve(field, defs)["description"].as_str());
            if let Some(doc) = doc {
                out.push_str(&indented(doc, 4));
            }
            out.push('\n');
        }
    }
    out
}

/// Follows `$ref` and the `anyOf [T, null]` an `Option<T>` becomes.
fn resolve<'a>(node: &'a Value, defs: &'a Map<String, Value>) -> &'a Value {
    let mut node = node;
    loop {
        if let Some(target) = node["$ref"].as_str() {
            match defs.get(target.trim_start_matches("#/$defs/")) {
                Some(def) => node = def,
                None => return node,
            }
        } else if let Some(variants) = node["anyOf"].as_array() {
            match variants.iter().find(|v| v["type"] != "null") {
                Some(variant) => node = variant,
                None => return node,
            }
        } else {
            return node;
        }
    }
}

fn type_name(node: &Value, defs: &Map<String, Value>) -> String {
    let resolved = resolve(node, defs);
    if enum_values(resolved).is_some() {
        return "string".to_owned();
    }
    let kind = match &resolved["type"] {
        Value::String(kind) => kind.as_str(),
        Value::Array(kinds) => kinds
            .iter()
            .filter_map(Value::as_str)
            .find(|kind| *kind != "null")
            .unwrap_or("Object"),
        _ => "object",
    };
    match kind {
        "array" => format!("[]{}", type_name(&resolved["items"], defs)),
        "object" => "Object".to_owned(),
        other => other.to_owned(),
    }
}

/// The values of an enum, as schemars writes one: `oneOf` of `const`s.
fn enum_values(node: &Value) -> Option<Vec<(&str, Option<&str>)>> {
    let variants = node["oneOf"].as_array()?;
    variants
        .iter()
        .map(|v| Some((v["const"].as_str()?, v["description"].as_str())))
        .collect()
}

fn indented(text: &str, spaces: usize) -> String {
    let pad = " ".repeat(spaces);
    text.lines()
        .map(|line| {
            if line.is_empty() {
                "\n".to_owned()
            } else {
                format!("{pad}{line}\n")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "title": "Project",
            "description": "A resource as the API serves it.",
            "properties": {
                "spec": {"$ref": "#/$defs/ProjectSpec", "description": "Desired state."},
            },
            "$defs": {
                "ProjectSpec": {
                    "type": "object",
                    "required": ["title"],
                    "properties": {
                        "title": {"type": "string", "description": "Display name."},
                        "highlight": {"type": ["integer", "null"], "description": "Position."},
                        "status": {"$ref": "#/$defs/ProjectStatus"},
                        "tags": {"type": "array", "items": {"type": "string"}},
                        "links": {"$ref": "#/$defs/ProjectLinks"},
                        "banner": {"anyOf": [{"$ref": "#/$defs/ProjectLinks"}, {"type": "null"}]},
                    }
                },
                "ProjectStatus": {
                    "description": "Where a project stands.",
                    "oneOf": [
                        {"const": "active", "type": "string", "description": "In use."},
                        {"const": "wip", "type": "string"},
                    ]
                },
                "ProjectLinks": {"type": "object", "description": "Links.", "properties": {}},
            }
        })
    }

    fn at(path: &[&str]) -> String {
        let schema = schema();
        let defs = schema["$defs"].as_object().unwrap().clone();
        let mut node = &schema;
        for part in path {
            node = &resolve(node, &defs)["properties"][*part];
        }
        explain("Project", path, node, &defs)
    }

    #[test]
    fn explains_a_kind_and_its_fields() {
        let spec = at(&["spec"]);
        assert!(
            spec.starts_with("KIND:     Project\nFIELD:    spec <Object>\n"),
            "{spec}"
        );
        assert!(
            spec.contains("DESCRIPTION:\n    Desired state.\n"),
            "{spec}"
        );
        assert!(
            spec.contains("  title\t<string> -required-\n    Display name.\n"),
            "{spec}"
        );
        assert!(spec.contains("  highlight\t<integer>\n"), "{spec}");
        assert!(
            spec.contains("  status\t<string>\n    Where a project stands.\n"),
            "{spec}"
        );
        assert!(spec.contains("  tags\t<[]string>\n"), "{spec}");
        assert!(spec.contains("  banner\t<Object>\n    Links.\n"), "{spec}");
    }

    #[test]
    fn explains_enum_values() {
        let status = at(&["spec", "status"]);
        assert!(
            status.contains("VALUES:\n    active\n        In use.\n    wip\n"),
            "{status}"
        );
    }
}
