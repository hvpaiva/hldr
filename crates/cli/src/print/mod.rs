//! `-o` formats, shared by every command that prints resources.

pub mod jsonpath;

use std::io::{self, Write};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use hldr_core::api::ApiResource;
use serde_json::{Value, json};

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
                table(out, headers, rows, no_headers)?;
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
            table(out, headers, rows, no_headers)
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
            writeln!(
                out,
                "{}",
                serde_json::to_string_pretty(&value).map_err(io::Error::other)?
            )
        }
        Output::Yaml => {
            let value = combined(groups);
            write!(
                out,
                "{}",
                serde_saphyr::to_string(&value).map_err(io::Error::other)?
            )
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

/// Left-aligned columns three spaces apart, as kubectl prints them.
pub fn table(
    out: &mut dyn Write,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
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
    for line in &lines {
        let mut text = String::new();
        for (i, cell) in line.iter().enumerate() {
            if i + 1 < line.len() {
                text.push_str(&format!("{cell:<width$}   ", width = widths[i]));
            } else {
                text.push_str(cell);
            }
        }
        writeln!(out, "{}", text.trim_end())?;
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
        print(&mut out, groups, &output.parse().unwrap(), no_headers).unwrap();
        String::from_utf8(out).unwrap()
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
