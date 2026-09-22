//! `hldr`: administration of hvpaiva.dev in the kubectl shape.

use std::io::{self, BufWriter, IsTerminal, Write};
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod client;
mod cmd;
mod color;
mod config;
mod content;
mod discovery;
mod editor;
mod github;
mod print;
#[cfg(test)]
mod stub;

use client::Client;
use color::Term;
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
    /// Disable colored output.
    #[arg(long, global = true)]
    plain: bool,
    /// Color even when output is not a terminal, with this many colors: auto,
    /// basic, 256 or truecolor; `none` never colors.
    #[arg(
        long,
        global = true,
        env = "HLDR_FORCE_COLORS",
        value_name = "LEVEL",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "auto"
    )]
    force_colors: Option<String>,
    /// Use the light variant of the color preset.
    #[arg(long, global = true, env = "HLDR_LIGHT_BACKGROUND")]
    light_background: bool,
    /// Color preset: auto, dark, light, none, or protanopia, deuteranopia or
    /// tritanopia, alone or with -dark or -light; overrides `color.preset`.
    #[arg(long, global = true, env = "HLDR_COLOR_PRESET", value_name = "PRESET")]
    color_preset: Option<String>,
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    fn color(&self) -> color::Options {
        color::Options {
            plain: self.plain,
            force: self.force_colors.clone(),
            light_background: self.light_background,
            preset: self.color_preset.clone(),
        }
    }
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
    /// Edit a resource's file in $EDITOR and commit it
    Edit(cmd::edit::Args),
    /// Commit files to the content repository, as they are
    Apply(cmd::apply::Args),
    /// Show how applying files would change the content repository
    Diff(cmd::diff::Args),
    /// Change fields of a resource with a JSON merge patch
    Patch(cmd::patch::Args),
    /// Remove resources from the content repository
    Delete(cmd::delete::Args),
    /// Have the server fetch its content now, or show where it stands
    Sync(cmd::sync::Args),
    /// Check content files or a checkout offline, as the server would index them
    Validate(cmd::validate::Args),
    /// Print the client, server and content versions
    Version(cmd::version::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let env = Env::from_process();
    let file = env.config_file();
    let setup = Config::load(file.as_deref()).and_then(|config| {
        let term = Term::new(
            &cli.color(),
            &env.color,
            config.color.as_ref(),
            io::stdout().is_terminal(),
            io::stderr().is_terminal(),
        )?;
        Ok((config, term))
    });
    let (config, term) = match setup {
        Ok(setup) => setup,
        Err(err) => {
            eprintln!("error: {err:#}");
            return ExitCode::FAILURE;
        }
    };
    let mut out = BufWriter::new(io::stdout().lock());
    let result = run(&cli, &env, &config, &term, &mut out).and_then(|ok| {
        out.flush()?;
        Ok(ok)
    });
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) if broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            term.error(&format!("{err:#}"));
            // `diff` answers with 1 itself, so its failures take 2, as kubectl's do.
            match cli.command {
                Command::Diff(_) => ExitCode::from(2),
                _ => ExitCode::FAILURE,
            }
        }
    }
}

/// Returns whether every requested resource was found; errors that stop the
/// command come back as `Err`.
fn run(cli: &Cli, env: &Env, config: &Config, term: &Term, out: &mut dyn Write) -> Result<bool> {
    // Offline: no server is needed, as in CI; the config file only colors.
    if let Command::Validate(args) = &cli.command {
        return cmd::validate::run(out, term, args);
    }
    let file = env.config_file();
    let connect = || -> Result<Client> {
        Ok(Client::new(config::server(
            cli.server.as_deref(),
            config,
            file.as_deref(),
        )?))
    };
    let cache = env.cache_dir();
    let writer = |client| -> Result<cmd::Writer<'_>> {
        cmd::Writer::new(
            client,
            Catalog::new(client, cache.as_deref()),
            github::API,
            env.editor.clone(),
            config::github_token(env, config)?,
        )
    };
    match &cli.command {
        Command::Edit(args) => cmd::edit::run(out, term, &mut writer(&connect()?)?, args),
        Command::Apply(args) => cmd::apply::run(out, term, &mut writer(&connect()?)?, args),
        Command::Diff(args) => cmd::diff::run(out, term, &mut writer(&connect()?)?, args),
        Command::Patch(args) => cmd::patch::run(out, term, &mut writer(&connect()?)?, args),
        Command::Delete(args) => cmd::delete::run(out, term, &mut writer(&connect()?)?, args),
        Command::Sync(args) => {
            let client = connect()?;
            let token = || config::github_token(env, config);
            let mut catalog = Catalog::new(&client, cache.as_deref());
            cmd::sync::run(out, term, &client, &mut catalog, github::API, &token, args)
        }
        Command::Version(args) if args.client_only() => cmd::version::run(out, term, None, args),
        Command::Version(args) => cmd::version::run(out, term, Some(&connect()?), args),
        Command::Validate(args) => cmd::validate::run(out, term, args),
        Command::Get(args) => {
            let client = connect()?;
            cmd::get::run(
                out,
                term,
                &client,
                &mut Catalog::new(&client, cache.as_deref()),
                args,
            )
        }
        Command::Describe(args) => {
            let client = connect()?;
            cmd::describe::run(
                out,
                term,
                &client,
                &mut Catalog::new(&client, cache.as_deref()),
                args,
            )
        }
        Command::Explain(args) => {
            let client = connect()?;
            cmd::explain::run(
                out,
                term,
                &mut Catalog::new(&client, cache.as_deref()),
                args,
            )
        }
        Command::ApiResources(args) => {
            let client = connect()?;
            cmd::api_resources::run(
                out,
                term,
                &mut Catalog::new(&client, cache.as_deref()),
                args,
            )
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
