use std::io::Write;

use anyhow::{Result, bail};
use serde_json::json;

use crate::color::{self, Term};
use crate::discovery::Catalog;
use crate::print::{self, Output};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// table, wide, json, yaml or name
    #[arg(short, long, default_value = "table")]
    output: String,
    /// Leave out the header row of tables.
    #[arg(long)]
    no_headers: bool,
}

/// Always asks the server, and refreshes the cache with the answer.
pub fn run(
    out: &mut dyn Write,
    term: &Term,
    catalog: &mut Catalog<'_>,
    args: &Args,
) -> Result<bool> {
    let output: Output = args.output.parse()?;
    let resources = &catalog.refresh()?.resources;
    match output {
        Output::Table | Output::Wide => {
            let wide = matches!(output, Output::Wide);
            let mut headers = vec!["NAME", "SHORTNAMES", "SINGLETON", "KIND"];
            if wide {
                headers.push("VERBS");
            }
            let rows = resources
                .iter()
                .map(|r| {
                    let mut row = vec![
                        r.name.clone(),
                        r.short_names.join(","),
                        r.singleton.to_string(),
                        r.kind.clone(),
                    ];
                    if wide {
                        row.push(r.verbs.join(","));
                    }
                    row
                })
                .collect();
            let headers = headers.into_iter().map(str::to_owned).collect();
            print::table(out, term.out(), headers, rows, &[], args.no_headers)?;
        }
        Output::Name => {
            for resource in resources {
                writeln!(out, "{}", resource.name)?;
            }
        }
        Output::Json => {
            let list = json!({"kind": "APIResourceList", "items": resources});
            let text = serde_json::to_string_pretty(&list)?;
            writeln!(out, "{}", color::json(term.out(), &text))?;
        }
        Output::Yaml => {
            let list = json!({"kind": "APIResourceList", "items": resources});
            let text = serde_saphyr::to_string(&list)?;
            write!(out, "{}", color::yaml(term.out(), &text))?;
        }
        Output::JsonPath(_) | Output::CustomColumns(_) => {
            bail!("api-resources prints table, wide, json, yaml or name");
        }
    }
    Ok(true)
}
