//! `hldr`: administration of hvpaiva.dev in the kubectl shape.

use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod client;
mod cmd;
mod config;
mod discovery;
mod print;
#[cfg(test)]
mod stub;

use client::Client;
use config::{Config, Env};
use discovery::Catalog;

#[derive(Parser)]
#[command(
    name = "hldr",
    version = hldr_core::VERSION,
    about = "Command-line interface for hvpaiva.dev",
    propagate_version = true
)]
struct Cli {
    /// Base URL of the private API; overrides `server:` in the config file.
    #[arg(long, global = true, env = "HLDR_SERVER", value_name = "URL")]
    server: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Display one or many resources
    Get(cmd::get::Args),
    /// Show the details of one or many resources
    Describe(cmd::describe::Args),
    /// Document a resource type and its fields
    Explain(cmd::explain::Args),
    /// List the resource types the server has
    ApiResources(cmd::api_resources::Args),
    /// Print the client, server and content versions
    Version(cmd::version::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let env = Env::from_process();
    let mut out = BufWriter::new(io::stdout().lock());
    let result = run(&cli, &env, &mut out).and_then(|ok| {
        out.flush()?;
        Ok(ok)
    });
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) if broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Returns whether every requested resource was found; errors that stop the
/// command come back as `Err`.
fn run(cli: &Cli, env: &Env, out: &mut dyn Write) -> Result<bool> {
    let connect = || -> Result<Client> {
        let file = env.config_file();
        let config = Config::load(file.as_deref())?;
        Ok(Client::new(config::server(
            cli.server.as_deref(),
            &config,
            file.as_deref(),
        )?))
    };
    let cache = env.cache_dir();
    match &cli.command {
        Command::Version(args) if args.client_only() => cmd::version::run(out, None, args),
        Command::Version(args) => cmd::version::run(out, Some(&connect()?), args),
        Command::Get(args) => {
            let client = connect()?;
            cmd::get::run(
                out,
                &client,
                &mut Catalog::new(&client, cache.as_deref()),
                args,
            )
        }
        Command::Describe(args) => {
            let client = connect()?;
            cmd::describe::run(
                out,
                &client,
                &mut Catalog::new(&client, cache.as_deref()),
                args,
            )
        }
        Command::Explain(args) => {
            let client = connect()?;
            cmd::explain::run(out, &mut Catalog::new(&client, cache.as_deref()), args)
        }
        Command::ApiResources(args) => {
            let client = connect()?;
            cmd::api_resources::run(out, &mut Catalog::new(&client, cache.as_deref()), args)
        }
    }
}

/// `hldr get p | head` closes stdout early; that is not a failure.
fn broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|io| io.kind() == io::ErrorKind::BrokenPipe)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
