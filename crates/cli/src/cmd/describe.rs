//! A resource for people: every field labeled and aligned, nested fields
//! indented, markdown bodies shown as they are written.

use std::io::Write;

use anyhow::Result;
use serde_json::{Map, Value};

use crate::client::Client;
use crate::color::{Painter, Role, Term};
use crate::discovery::Catalog;
use crate::print::jsonpath::plain;
use crate::print::{self, Fetched, Output};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// TYPE, TYPE NAME..., or TYPE/NAME...
    #[arg(value_name = "TYPE[/NAME]", required = true)]
    targets: Vec<String>,
}

pub fn run(
    out: &mut dyn Write,
    term: &Term,
    client: &Client,
    catalog: &mut Catalog<'_>,
    args: &Args,
) -> Result<bool> {
    let (fetched, complete) = super::fetch(term, client, catalog, &args.targets, "describe")?;
    let mut first = true;
    for group in &fetched {
        for item in group.items() {
            if !first {
                writeln!(out)?;
            }
            first = false;
            write!(out, "{}", describe(term.out(), item))?;
            if item["kind"] == "Server" {
                write!(out, "{}", events(term.out(), client, catalog)?)?;
            }
        }
    }
    Ok(complete)
}

/// How many of the latest events `describe server` shows.
const RECENT_EVENTS: usize = 10;

/// The latest events, as `kubectl describe` ends with them, in the table
/// and colors of `hldr get events`.
fn events(paint: Painter<'_>, client: &Client, catalog: &mut Catalog<'_>) -> Result<String> {
    let resource = catalog.resolve("events")?;
    let mut list = client.get(&format!("/api/v1/{}", resource.name))?;
    let label = format!("{}:", paint.nth(Role::DescribeKey, 0, "Events"));
    let items = list["items"]
        .as_array_mut()
        .map(std::mem::take)
        .unwrap_or_default();
    if items.is_empty() {
        return Ok(format!(
            "{label}  {}\n",
            paint.paint(Role::DataNull, "<none>")
        ));
    }
    let recent = items[items.len().saturating_sub(RECENT_EVENTS)..].to_vec();
    list["items"] = Value::Array(recent);
    let mut table = Vec::new();
    print::print(
        &mut table,
        paint,
        &[Fetched {
            resource,
            value: list,
        }],
        &Output::Table,
        false,
    )?;
    let mut out = format!("{label}\n");
    for line in String::from_utf8_lossy(&table).lines() {
        out.push_str(&format!("  {line}\n"));
    }
    Ok(out)
}

pub fn describe(paint: Painter<'_>, item: &Value) -> String {
    let mut fields: Vec<(String, &Value)> = Vec::new();
    if let Some(metadata) = item["metadata"].as_object() {
        fields.extend(metadata.iter().map(|(key, value)| (label(key), value)));
    }
    let after_name = usize::from(item["metadata"]["name"].is_string());
    fields.insert(after_name, ("Kind".to_owned(), &item["kind"]));
    // What the server keeps, such as the server itself, declares no spec.
    if item.get("spec").is_some() {
        fields.push(("Spec".to_owned(), &item["spec"]));
    }
    if item["status"]
        .as_object()
        .is_some_and(|status| !status.is_empty())
    {
        fields.push(("Status".to_owned(), &item["status"]));
    }
    let mut out = String::new();
    write_fields(paint, &mut out, &fields, 0);
    out
}

fn write_fields(paint: Painter<'_>, out: &mut String, fields: &[(String, &Value)], indent: usize) {
    let width = fields
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0)
        + 4;
    let pad = " ".repeat(indent);
    for (label, value) in fields {
        let head = format!("{pad}{}:", paint.nth(Role::DescribeKey, indent / 2, label));
        match value {
            Value::Object(map) if !map.is_empty() => {
                out.push_str(&format!("{head}\n"));
                write_fields(paint, out, &labeled(map), indent + 2);
            }
            Value::Array(items) if items.iter().any(Value::is_object) => {
                out.push_str(&format!("{head}\n"));
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push('\n');
                    }
                    match item.as_object() {
                        Some(map) => write_fields(paint, out, &labeled(map), indent + 2),
                        None => out.push_str(&format!("{pad}  {}\n", paint.value(&plain(item)))),
                    }
                }
            }
            Value::String(text) if text.trim_end().contains('\n') => {
                out.push_str(&format!("{head}\n"));
                for line in text.trim_matches('\n').lines() {
                    if line.is_empty() {
                        out.push('\n');
                    } else {
                        let line = paint.paint(Role::DataString, line);
                        out.push_str(&format!("{pad}    {line}\n"));
                    }
                }
            }
            scalar => {
                let spaces = (width + indent).saturating_sub(indent + label.chars().count() + 1);
                out.push_str(&head);
                out.push_str(&" ".repeat(spaces));
                out.push_str(&inline(paint, scalar));
                out.push('\n');
            }
        }
    }
}

fn labeled(map: &Map<String, Value>) -> Vec<(String, &Value)> {
    map.iter().map(|(key, value)| (label(key), value)).collect()
}

fn inline(paint: Painter<'_>, value: &Value) -> String {
    match value {
        Value::Null => paint.paint(Role::DataNull, "<none>"),
        Value::Object(map) if map.is_empty() => paint.paint(Role::DataNull, "<none>"),
        Value::Array(items) if items.is_empty() => paint.paint(Role::DataNull, "<none>"),
        Value::Array(items) => items
            .iter()
            .map(|item| paint.value(&plain(item)))
            .collect::<Vec<_>>()
            .join(", "),
        // A string reads as a string even when it spells a number.
        Value::String(text) => paint.paint(Role::DataString, text.trim_end()),
        other => paint.value(&plain(other)),
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
            describe(Term::plain().out(), &project),
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
    fn colors_keys_by_depth_and_keeps_alignment() {
        let options = crate::color::Options {
            force: Some("basic".to_owned()),
            preset: Some("dark".to_owned()),
            ..crate::color::Options::default()
        };
        let term = Term::new(&options, &Default::default(), None, false, false).unwrap();
        let site = json!({"kind": "Site", "metadata": {}, "spec": {"title": "x", "n": 2, "on": true, "no": null}, "status": {}});
        assert_eq!(
            describe(term.out(), &site),
            "\x1b[96mKind\x1b[0m:   \x1b[93mSite\x1b[0m\n\
             \x1b[96mSpec\x1b[0m:\n  \
             \x1b[36mTitle\x1b[0m:   \x1b[93mx\x1b[0m\n  \
             \x1b[36mN\x1b[0m:       \x1b[35m2\x1b[0m\n  \
             \x1b[36mOn\x1b[0m:      \x1b[32mtrue\x1b[0m\n  \
             \x1b[36mNo\x1b[0m:      \x1b[90;3m<none>\x1b[0m\n"
        );
    }

    #[test]
    fn singletons_have_no_name() {
        let site = json!({"kind": "Site", "metadata": {"updated_at": "t"}, "spec": {"title": "x"}, "status": {}});
        assert!(
            describe(Term::plain().out(), &site)
                .starts_with("Kind:         Site\nUpdated At:   t\n"),
            "{}",
            describe(Term::plain().out(), &site)
        );
    }

    #[test]
    fn the_server_has_a_status_and_no_spec() {
        let server = json!({
            "kind": "Server", "metadata": {},
            "status": {"version": "4.2.0", "uptime": "3h2m", "github": null,
                       "database": {"bytes": 4096, "wal_bytes": 0}},
        });
        assert_eq!(
            describe(Term::plain().out(), &server),
            "\
Kind:     Server
Status:
  Version:    4.2.0
  Uptime:     3h2m
  Github:     <none>
  Database:
    Bytes:       4096
    Wal Bytes:   0
"
        );
    }

    fn events_stub() -> crate::stub::Stub {
        let stub = crate::stub::Stub::default();
        let mut events = crate::stub::resource("events", "event", "Event");
        events["columns"] = json!([
            {"name": "TYPE", "json_path": ".type", "wide": false,
             "values": ["Normal", "Warning"], "ok": ["Normal"]},
            {"name": "REASON", "json_path": ".reason", "wide": false},
            {"name": "FIRST SEEN", "json_path": ".first_at", "wide": true},
        ]);
        stub.extra.lock().unwrap().push(events);
        stub
    }

    #[test]
    fn the_server_ends_with_its_latest_events() {
        let stub = events_stub();
        let items: Vec<Value> = (1..=12)
            .map(|n| {
                json!({"kind": "Event", "type": if n == 12 { "Warning" } else { "Normal" },
                            "reason": format!("R{n}"), "first_at": "t"})
            })
            .collect();
        stub.script_events([json!({"kind": "EventList", "seq": 12, "items": items})]);
        let client = Client::new(stub.serve());
        let mut catalog = Catalog::new(&client, None);
        let text = events(Term::plain().out(), &client, &mut catalog).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Events:");
        assert_eq!(lines[1], "  TYPE      REASON");
        assert_eq!(lines[2], "  Normal    R3");
        assert_eq!(lines[11], "  Warning   R12");
        assert_eq!(lines.len(), 2 + RECENT_EVENTS);
    }

    #[test]
    fn a_server_without_events_says_so() {
        let stub = events_stub();
        stub.script_events([json!({"kind": "EventList", "seq": 0, "items": []})]);
        let client = Client::new(stub.serve());
        let mut catalog = Catalog::new(&client, None);
        let text = events(Term::plain().out(), &client, &mut catalog).unwrap();
        assert_eq!(text, "Events:  <none>\n");
    }

    #[test]
    fn labels_read_as_words() {
        assert_eq!(label("updated_at"), "Updated At");
        assert_eq!(label("fg_dim"), "Fg Dim");
        assert_eq!(label("accent2"), "Accent2");
    }
}
