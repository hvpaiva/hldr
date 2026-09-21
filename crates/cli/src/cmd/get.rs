use std::io::Write;

use anyhow::Result;

use crate::client::Client;
use crate::discovery::Catalog;
use crate::print::{self, Output};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// TYPE, TYPE NAME..., or TYPE/NAME...
    #[arg(value_name = "TYPE[/NAME]", required = true)]
    targets: Vec<String>,
    /// table, wide, json, yaml, name, jsonpath=TEMPLATE or
    /// custom-columns=HEADER:PATH,...
    #[arg(short, long, default_value = "table")]
    output: String,
    /// Leave out the header row of tables.
    #[arg(long)]
    no_headers: bool,
}

pub fn run(
    out: &mut dyn Write,
    client: &Client,
    catalog: &mut Catalog<'_>,
    args: &Args,
) -> Result<bool> {
    let output: Output = args.output.parse()?;
    let (fetched, complete) = super::fetch(client, catalog, &args.targets, "get")?;
    print::print(out, &fetched, &output, args.no_headers)?;
    Ok(complete)
}
