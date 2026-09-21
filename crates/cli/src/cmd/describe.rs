//! A resource for people: every field labeled and aligned, nested fields
//! indented, markdown bodies shown as they are written.

use std::io::Write;

use anyhow::Result;
use serde_json::{Map, Value};

use crate::client::Client;
use crate::discovery::Catalog;
use crate::print::jsonpath::plain;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// TYPE, TYPE NAME..., or TYPE/NAME...
    #[arg(value_name = "TYPE[/NAME]", required = true)]
    targets: Vec<String>,
}

pub fn run(
    out: &mut dyn Write,
    client: &Client,
    catalog: &mut Catalog<'_>,
    args: &Args,
) -> Result<bool> {
    let (fetched, complete) = super::fetch(client, catalog, &args.targets, "describe")?;
    let mut first = true;
    for group in &fetched {
        for item in group.items() {
            if !first {
                writeln!(out)?;
            }
            first = false;
            write!(out, "{}", describe(item))?;
        }
    }
    Ok(complete)
}

pub fn describe(item: &Value) -> String {
    let mut fields: Vec<(String, &Value)> = Vec::new();
    if let Some(metadata) = item["metadata"].as_object() {
        fields.extend(metadata.iter().map(|(key, value)| (label(key), value)));
    }
    let after_name = usize::from(item["metadata"]["name"].is_string());
    fields.insert(after_name, ("Kind".to_owned(), &item["kind"]));
    fields.push(("Spec".to_owned(), &item["spec"]));
    if item["status"]
        .as_object()
        .is_some_and(|status| !status.is_empty())
    {
        fields.push(("Status".to_owned(), &item["status"]));
    }
    let mut out = String::new();
    write_fields(&mut out, &fields, 0);
    out
}

fn write_fields(out: &mut String, fields: &[(String, &Value)], indent: usize) {
    let width = fields
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0)
        + 4;
    let pad = " ".repeat(indent);
    for (label, value) in fields {
        let head = format!("{pad}{label}:");
        match value {
            Value::Object(map) if !map.is_empty() => {
                out.push_str(&format!("{head}\n"));
                write_fields(out, &labeled(map), indent + 2);
            }
            Value::Array(items) if items.iter().any(Value::is_object) => {
                out.push_str(&format!("{head}\n"));
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push('\n');
                    }
                    match item.as_object() {
                        Some(map) => write_fields(out, &labeled(map), indent + 2),
                        None => out.push_str(&format!("{pad}  {}\n", plain(item))),
                    }
                }
            }
            Value::String(text) if text.trim_end().contains('\n') => {
                out.push_str(&format!("{head}\n"));
                for line in text.trim_matches('\n').lines() {
                    if line.is_empty() {
                        out.push('\n');
                    } else {
                        out.push_str(&format!("{pad}    {line}\n"));
                    }
                }
            }
            scalar => {
                let column = width + indent;
                out.push_str(&format!("{head:<column$}{}\n", inline(scalar)));
            }
        }
    }
}

fn labeled(map: &Map<String, Value>) -> Vec<(String, &Value)> {
    map.iter().map(|(key, value)| (label(key), value)).collect()
}

fn inline(value: &Value) -> String {
    match value {
        Value::Null => "<none>".to_owned(),
        Value::Object(map) if map.is_empty() => "<none>".to_owned(),
        Value::Array(items) if items.is_empty() => "<none>".to_owned(),
        Value::Array(items) => items.iter().map(plain).collect::<Vec<_>>().join(", "),
        Value::String(text) => text.trim_end().to_owned(),
        other => plain(other),
    }
}

/// `updated_at` reads as `Updated At`.
fn label(key: &str) -> String {
    key.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn describes_nested_fields_and_bodies() {
        let project = json!({
            "kind": "Project",
            "metadata": {"name": "atlas", "updated_at": "2026-01-02T00:00:00Z"},
            "spec": {
                "title": "Atlas",
                "highlight": null,
                "tags": ["rust", "cubesat"],
                "links": {"repo": "https://github.com/example/atlas"},
                "assets": [{"path": "a.png", "caption": "one"}, {"path": "b.png"}],
                "body": "\n## What it is\n\nA fixture.\n",
            },
            "status": {},
        });
        assert_eq!(
            describe(&project),
            "\
Name:         atlas
Kind:         Project
Updated At:   2026-01-02T00:00:00Z
Spec:
  Title:       Atlas
  Highlight:   <none>
  Tags:        rust, cubesat
  Links:
    Repo:   https://github.com/example/atlas
  Assets:
    Path:      a.png
    Caption:   one

    Path:   b.png
  Body:
      ## What it is

      A fixture.
"
        );
    }

    #[test]
    fn singletons_have_no_name() {
        let site = json!({"kind": "Site", "metadata": {"updated_at": "t"}, "spec": {"title": "x"}, "status": {}});
        assert!(
            describe(&site).starts_with("Kind:         Site\nUpdated At:   t\n"),
            "{}",
            describe(&site)
        );
    }

    #[test]
    fn labels_read_as_words() {
        assert_eq!(label("updated_at"), "Updated At");
        assert_eq!(label("fg_dim"), "Fg Dim");
        assert_eq!(label("accent2"), "Accent2");
    }
}
