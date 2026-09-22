//! `-o` formats, shared by every command that prints resources.

pub mod jsonpath;

use std::io::{self, Write};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use hldr_core::api::{ApiResource, Column};
use serde_json::{Value, json};

use crate::color::{self, Painter, Role};
use jsonpath::{Path, Template, plain};

#[derive(Debug)]
pub enum Output {
    Table,
    Wide,
    Json,
    Yaml,
    Name,
    JsonPath(Template),
    CustomColumns(Vec<(String, Path)>),
}

impl FromStr for Output {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let (format, argument) = value.split_once('=').unwrap_or((value, ""));
        Ok(match format {
            "table" if argument.is_empty() => Self::Table,
            "wide" if argument.is_empty() => Self::Wide,
            "json" if argument.is_empty() => Self::Json,
            "yaml" if argument.is_empty() => Self::Yaml,
            "name" if argument.is_empty() => Self::Name,
            "jsonpath" => Self::JsonPath(Template::parse(argument)?),
            "custom-columns" => Self::CustomColumns(custom_columns(argument)?),
            _ => bail!(
                "unknown output {value:?}: use table, wide, json, yaml, name, \
                 jsonpath=TEMPLATE or custom-columns=HEADER:PATH,..."
            ),
        })
    }
}

fn custom_columns(spec: &str) -> Result<Vec<(String, Path)>> {
    if spec.is_empty() {
        bail!("custom-columns needs HEADER:PATH pairs, such as NAME:.metadata.name");
    }
    spec.split(',')
        .map(|column| {
            let (header, path) = column
                .split_once(':')
                .with_context(|| format!("custom column {column:?} is not HEADER:PATH"))?;
            Ok((header.to_owned(), Path::parse(path)?))
        })
        .collect()
}

/// Resources of one type as the API returned them: one object, or a list.
pub struct Fetched {
    pub resource: ApiResource,
    pub value: Value,
}

impl Fetched {
    pub fn items(&self) -> Vec<&Value> {
        items(&self.value)
    }
}

/// The objects in a value: a `*List`'s items, or the value itself.
pub fn items(value: &Value) -> Vec<&Value> {
    let is_list = value["kind"]
        .as_str()
        .is_some_and(|kind| kind.ends_with("List"));
    match value.get("items").and_then(Value::as_array) {
        Some(items) if is_list => items.iter().collect(),
        _ => vec![value],
    }
}

pub fn print(
    out: &mut dyn Write,
    paint: Painter<'_>,
    groups: &[Fetched],
    output: &Output,
    no_headers: bool,
) -> io::Result<()> {
    match output {
        Output::Table | Output::Wide => {
            let wide = matches!(output, Output::Wide);
            for (i, group) in groups.iter().enumerate() {
                if i > 0 {
                    writeln!(out)?;
                }
                let columns: Vec<_> = group
                    .resource
                    .columns
                    .iter()
                    .filter(|column| wide || !column.wide)
                    .collect();
                let paths: Vec<Option<Path>> = columns
                    .iter()
                    .map(|column| Path::parse(&column.json_path).ok())
                    .collect();
                let rows = group
                    .items()
                    .into_iter()
                    .map(|item| {
                        paths
                            .iter()
                            .map(|path| {
                                path.as_ref()
                                    .map_or_else(|| "<none>".to_owned(), |p| cell(&p.values(item)))
                            })
                            .collect()
                    })
                    .collect();
                let headers = columns.iter().map(|column| column.name.clone()).collect();
                table(out, paint, headers, rows, &columns, no_headers)?;
            }
            Ok(())
        }
        Output::CustomColumns(columns) => {
            let rows = groups
                .iter()
                .flat_map(Fetched::items)
                .map(|item| {
                    columns
                        .iter()
                        .map(|(_, path)| cell(&path.values(item)))
                        .collect()
                })
                .collect();
            let headers = columns.iter().map(|(header, _)| header.clone()).collect();
            table(out, paint, headers, rows, &[], no_headers)
        }
        Output::Name => {
            for group in groups {
                for item in group.items() {
                    writeln!(out, "{}", name_of(&group.resource, item))?;
                }
            }
            Ok(())
        }
        Output::Json => {
            let value = combined(groups);
            let text = serde_json::to_string_pretty(&value).map_err(io::Error::other)?;
            writeln!(out, "{}", color::json(paint, &text))
        }
        Output::Yaml => {
            let value = combined(groups);
            let text = serde_saphyr::to_string(&value).map_err(io::Error::other)?;
            write!(out, "{}", color::yaml(paint, &text))
        }
        Output::JsonPath(template) => write!(out, "{}", template.render(&combined(groups))),
    }
}

/// What `-o json` and friends print: the fetched value itself, or a `List`
/// when the command fetched more than one type.
fn combined(groups: &[Fetched]) -> Value {
    match groups {
        [one] => one.value.clone(),
        many => json!({
            "kind": "List",
            "items": many.iter().flat_map(Fetched::items).cloned().collect::<Vec<_>>(),
        }),
    }
}

/// `project/atlas`, or just `site` for a singleton.
pub fn name_of(resource: &ApiResource, item: &Value) -> String {
    match item["metadata"]["name"].as_str() {
        Some(name) if !resource.singleton => format!("{}/{name}", resource.singular),
        _ => resource.singular.clone(),
    }
}

fn cell(values: &[&Value]) -> String {
    let text = match values {
        [] => return "<none>".to_owned(),
        [Value::Null] => return "<none>".to_owned(),
        [Value::Array(items)] if items.is_empty() => return "<none>".to_owned(),
        [Value::Array(items)] => items.iter().map(plain).collect::<Vec<_>>().join(","),
        [one] => plain(one),
        many => many.iter().map(|v| plain(v)).collect::<Vec<_>>().join(","),
    };
    text.lines().next().unwrap_or_default().to_owned()
}

/// Left-aligned columns three spaces apart, as kubectl prints them; colored
/// as kubecolor colors them, the header in one style and each column in the
/// next of `table.columns`, with `<none>` muted. A cell of an enum column in
/// `specs`, by position, is a status: good when it is one of the column's
/// `ok` values, a warning when it is another of its values.
pub fn table(
    out: &mut dyn Write,
    paint: Painter<'_>,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    specs: &[&Column],
    no_headers: bool,
) -> io::Result<()> {
    let mut lines = Vec::with_capacity(rows.len() + 1);
    if !no_headers {
        lines.push(headers);
    }
    lines.extend(rows);
    let columns = lines.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|i| {
            lines
                .iter()
                .filter_map(|line| line.get(i))
                .map(|c| c.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    for (n, line) in lines.iter().enumerate() {
        let mut text = String::new();
        // Where each cell's text starts and ends in `text`.
        let mut cells = Vec::with_capacity(line.len());
        for (i, cell) in line.iter().enumerate() {
            cells.push((text.len(), text.len() + cell.len()));
            if i + 1 < line.len() {
                text.push_str(&format!("{cell:<width$}   ", width = widths[i]));
            } else {
                text.push_str(cell);
            }
        }
        let text = text.trim_end();
        if n == 0 && !no_headers {
            writeln!(out, "{}", paint.paint(Role::TableHeader, text))?;
            continue;
        }
        let mut colored = String::with_capacity(text.len() * 2);
        let mut at = 0;
        for (i, (start, end)) in cells.into_iter().enumerate() {
            let end = end.min(text.len());
            if start >= end {
                continue;
            }
            colored.push_str(&text[at..start]);
            let cell = &text[start..end];
            let spec = specs.get(i);
            let is = |values: fn(&Column) -> &[String]| {
                spec.is_some_and(|spec| values(spec).iter().any(|value| value == cell))
            };
            colored.push_str(&match cell {
                "<none>" => paint.paint(Role::DataNull, cell),
                _ if is(|spec| &spec.ok) => paint.paint(Role::StatusSuccess, cell),
                _ if is(|spec| &spec.values) => paint.paint(Role::StatusWarning, cell),
                _ => paint.nth(Role::TableColumns, i, cell),
            });
            at = end;
        }
        colored.push_str(&text[at..]);
        writeln!(out, "{colored}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projects() -> ApiResource {
        serde_json::from_value(json!({
            "name": "projects", "singular": "project", "short_names": ["p"], "kind": "Project",
            "singleton": false, "verbs": ["get"],
            "source": {"path": "projects/{name}.md", "format": "markdown"},
            "columns": [
                {"name": "NAME", "json_path": ".metadata.name", "wide": false},
                {"name": "HIGHLIGHT", "json_path": ".spec.highlight", "wide": false},
                {"name": "TAGS", "json_path": ".spec.tags", "wide": true},
            ],
        }))
        .unwrap()
    }

    fn site() -> ApiResource {
        let mut site = projects();
        site.name = "site".to_owned();
        site.singular = "site".to_owned();
        site.kind = "Site".to_owned();
        site.singleton = true;
        site
    }

    fn fetched() -> Fetched {
        Fetched {
            resource: projects(),
            value: json!({"kind": "ProjectList", "items": [
                {"kind": "Project", "metadata": {"name": "atlas"}, "spec": {"highlight": 1, "tags": ["rust", "cubesat"]}},
                {"kind": "Project", "metadata": {"name": "a-much-longer-name"}, "spec": {"highlight": null, "tags": []}},
            ]}),
        }
    }

    fn printed(groups: &[Fetched], output: &str, no_headers: bool) -> String {
        let mut out = Vec::new();
        let term = color::Term::plain();
        print(
            &mut out,
            term.out(),
            groups,
            &output.parse().unwrap(),
            no_headers,
        )
        .unwrap();
        String::from_utf8(out).unwrap()
    }

    fn colored(groups: &[Fetched], output: &str) -> String {
        let term = color::Term::basic();
        let mut out = Vec::new();
        print(
            &mut out,
            term.out(),
            groups,
            &output.parse().unwrap(),
            false,
        )
        .unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn colored_tables_keep_their_alignment() {
        assert_eq!(
            colored(&[fetched()], "table"),
            "\x1b[1mNAME                 HIGHLIGHT\x1b[0m\n\
             \x1b[37matlas\x1b[0m                \x1b[36m1\x1b[0m\n\
             \x1b[37ma-much-longer-name\x1b[0m   \x1b[90;3m<none>\x1b[0m\n"
        );
        let json = colored(&[fetched()], "json");
        assert!(json.contains("\x1b[96m\"kind\"\x1b[0m"), "{json}");
    }

    #[test]
    fn colored_tables_skip_empty_cells() {
        let term = color::Term::basic();
        let mut out = Vec::new();
        let row = |cells: &[&str]| cells.iter().map(|c| (*c).to_owned()).collect();
        table(
            &mut out,
            term.out(),
            row(&["A", "B", "C"]),
            vec![row(&["x", "", "z"]), row(&["y", "w", ""])],
            &[],
            true,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b[37mx\x1b[0m       \x1b[37mz\x1b[0m\n\x1b[37my\x1b[0m   \x1b[36mw\x1b[0m\n"
        );
    }

    #[test]
    fn enum_columns_color_as_statuses() {
        let mut resource = projects();
        resource.columns[1] = serde_json::from_value(json!({
            "name": "STATUS", "json_path": ".spec.status", "wide": false,
            "values": ["active", "wip"], "ok": ["active"],
        }))
        .unwrap();
        let group = Fetched {
            resource,
            value: json!({"kind": "ProjectList", "items": [
                {"metadata": {"name": "a"}, "spec": {"status": "active"}},
                {"metadata": {"name": "b"}, "spec": {"status": "wip"}},
                {"metadata": {"name": "c"}, "spec": {"status": "other"}},
            ]}),
        };
        assert_eq!(
            colored(&[group], "table"),
            "\x1b[1mNAME   STATUS\x1b[0m\n\
             \x1b[37ma\x1b[0m      \x1b[32mactive\x1b[0m\n\
             \x1b[37mb\x1b[0m      \x1b[33mwip\x1b[0m\n\
             \x1b[37mc\x1b[0m      \x1b[36mother\x1b[0m\n"
        );
    }

    #[test]
    fn tables_align_and_hide_wide_columns() {
        assert_eq!(
            printed(&[fetched()], "table", false),
            "NAME                 HIGHLIGHT\n\
             atlas                1\n\
             a-much-longer-name   <none>\n"
        );
        assert_eq!(
            printed(&[fetched()], "wide", true),
            "atlas                1        rust,cubesat\n\
             a-much-longer-name   <none>   <none>\n"
        );
    }

    #[test]
    fn names_are_type_slash_name() {
        assert_eq!(
            printed(&[fetched()], "name", false),
            "project/atlas\nproject/a-much-longer-name\n"
        );
        let one = Fetched {
            resource: site(),
            value: json!({"kind": "Site", "metadata": {}, "spec": {}}),
        };
        assert_eq!(printed(&[one], "name", false), "site\n");
    }

    #[test]
    fn structured_outputs_keep_the_list() {
        let json: Value = serde_json::from_str(&printed(&[fetched()], "json", false)).unwrap();
        assert_eq!(json["kind"], "ProjectList");
        let yaml = printed(&[fetched()], "yaml", false);
        assert!(yaml.contains("kind: ProjectList"), "{yaml}");

        let two = [fetched(), fetched()];
        let json: Value = serde_json::from_str(&printed(&two, "json", false)).unwrap();
        assert_eq!(json["kind"], "List");
        assert_eq!(json["items"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn templates_and_custom_columns() {
        assert_eq!(
            printed(&[fetched()], "jsonpath={.items[*].metadata.name}", false),
            "atlas a-much-longer-name"
        );
        assert_eq!(
            printed(
                &[fetched()],
                "custom-columns=N:.metadata.name,T:.spec.tags",
                false
            ),
            "N                    T\n\
             atlas                rust,cubesat\n\
             a-much-longer-name   <none>\n"
        );
    }

    #[test]
    fn rejects_unknown_outputs() {
        for bad in [
            "xml",
            "json=1",
            "custom-columns=",
            "custom-columns=NAME",
            "jsonpath={range}",
        ] {
            assert!(bad.parse::<Output>().is_err(), "{bad}");
        }
    }
}
