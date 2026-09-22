//! `hldr`: administration of hvpaiva.dev in the kubectl shape.

use std::ffi::OsString;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::process::ExitCode;

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

mod auth;
mod client;
mod cmd;
mod color;
mod config;
mod content;
mod discovery;
mod editor;
mod github;
mod pager;
mod print;
mod skew;
#[cfg(test)]
mod stub;

use client::Client;
use color::Term;
use config::{Config, Env};
use discovery::Catalog;
use pager::Pager;

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
    #[command(flatten)]
    color: ColorArgs,
    /// Page output on a terminal: auto, or never, the default; overrides
    /// `paging:` in the config file.
    #[arg(
        long,
        global = true,
        env = "HLDR_PAGING",
        value_name = "MODE",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "auto"
    )]
    paging: Option<String>,
    /// Do not page output; the same as --paging=never.
    #[arg(long, global = true)]
    no_paging: bool,
    /// Pager command, such as "less -RF", run without a shell; overrides
    /// `pager:` in the config file and PAGER.
    #[arg(long, global = true, env = "HLDR_PAGER", value_name = "COMMAND")]
    pager: Option<String>,
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    /// The pager, when paging is asked for and stdout is a terminal. `edit`
    /// is never paged: the editor has the terminal; nor is a watch, which
    /// never ends.
    fn pager(&self, env: &Env, config: &Config) -> Result<Option<Vec<String>>> {
        let settings = pager::Settings {
            no_paging: self.no_paging,
            paging: self.paging.as_deref(),
            config_paging: config.paging.as_deref(),
            pager: self.pager.as_deref(),
            config_pager: config.pager.as_deref(),
            env_pager: env.pager.as_deref(),
            path: env.path.as_deref(),
        };
        let command = settings.command()?;
        let unpaged = match &self.command {
            Command::Edit(_) | Command::Auth(_) => true,
            Command::Get(args) => args.watching(),
            _ => false,
        };
        let paged = io::stdout().is_terminal() && !unpaged;
        Ok(command.filter(|_| paged))
    }
}

#[derive(Parser, Default, Debug, PartialEq)]
struct ColorArgs {
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
}

impl ColorArgs {
    fn options(&self) -> color::Options {
        color::Options {
            plain: self.plain,
            force: self.force_colors.clone(),
            light_background: self.light_background,
            preset: self.color_preset.clone(),
        }
    }

    /// The color flags alone, read before the command line is parsed, since
    /// clap prints help and usage errors while it parses. Their values and
    /// environment variables are read by clap itself; anything it rejects
    /// here is reported by the full parse.
    fn prescan(args: impl IntoIterator<Item = OsString>) -> Self {
        let mut kept = vec![OsString::from("hldr")];
        let mut args = args.into_iter().skip(1);
        while let Some(arg) = args.next() {
            let Some(text) = arg.to_str() else { continue };
            match text {
                "--" => break,
                "--plain" | "--light-background" => kept.push(arg),
                "--color-preset" => {
                    kept.push(arg);
                    kept.extend(args.next());
                }
                _ if text == "--force-colors"
                    || text.starts_with("--force-colors=")
                    || text.starts_with("--color-preset=") =>
                {
                    kept.push(arg);
                }
                _ => {}
            }
        }
        Self::try_parse_from(kept).unwrap_or_default()
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
    /// List the commits of the content repository, or show one
    History(cmd::history::Args),
    /// Show the site's visits per day, or its top pages or referrers
    Top(cmd::top::Args),
    /// Check content files or a checkout offline, as the server would index them
    Validate(cmd::validate::Args),
    /// Print the client, server and content versions
    Version(cmd::version::Args),
    /// Log in to GitHub, which writes need, or see or forget the login
    Auth(cmd::auth::Args),
}

fn main() -> ExitCode {
    let env = Env::from_process();
    let file = env.config_file();
    let config = Config::load(file.as_deref());
    let term = |options: &color::Options, config: Option<&Config>| {
        Term::new(
            options,
            &env.color,
            config.and_then(|config| config.color.as_ref()),
            io::stdout().is_terminal(),
            io::stderr().is_terminal(),
        )
    };
    // A broken config or theme is reported after parsing; help is then plain.
    let early = term(
        &ColorArgs::prescan(std::env::args_os()).options(),
        config.as_ref().ok(),
    )
    .unwrap_or_else(|_| Term::plain());
    let matches = early.help(Cli::command()).get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|err| err.exit());
    let setup = config.and_then(|config| {
        let term = term(&cli.color.options(), Some(&config))?;
        let pager = cli.pager(&env, &config)?;
        Ok((config, term, pager))
    });
    let (config, term, pager) = match setup {
        Ok(setup) => setup,
        Err(err) => {
            eprintln!("error: {err:#}");
            return ExitCode::FAILURE;
        }
    };
    // While a pager may run, stderr waits for it to exit, so nothing lands
    // inside it or behind its screen.
    if pager.is_some() {
        term.hold();
    }
    let mut out = BufWriter::new(Pager::new(pager, &term));
    let result = run(&cli, &env, &config, &term, &mut out);
    let flushed = out.flush();
    let (pager, _) = out.into_parts();
    let closed = flushed.and(pager.finish());
    term.release();
    let result = result.and_then(|ok| {
        closed?;
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
    let catalog = |client| Catalog::new(client, cache.as_deref()).warn_on(term);
    let store = env.state_dir().map(auth::Store::new);
    let oauth = auth::OAuth::github();
    let token = || {
        auth::token(
            env.github_token.as_deref(),
            store.as_ref(),
            &oauth,
            term,
            hldr_core::github::now(),
        )
    };
    let writer = |client| -> Result<cmd::Writer<'_>> {
        cmd::Writer::new(
            client,
            catalog(client),
            github::API,
            env.editor.clone(),
            token()?,
        )
    };
    match &cli.command {
        Command::Auth(args) => {
            let context = cmd::auth::Context {
                oauth: &oauth,
                store: store.as_ref(),
                env_token: env.github_token.as_deref(),
                browser: env.browser,
                stdin_terminal: io::stdin().is_terminal(),
            };
            cmd::auth::run(out, term, &context, &connect, args)
        }
        Command::Edit(args) => cmd::edit::run(out, term, &mut writer(&connect()?)?, args),
        Command::Apply(args) => cmd::apply::run(out, term, &mut writer(&connect()?)?, args),
        Command::Diff(args) => cmd::diff::run(out, term, &mut writer(&connect()?)?, args),
        Command::Patch(args) => cmd::patch::run(out, term, &mut writer(&connect()?)?, args),
        Command::Delete(args) => cmd::delete::run(out, term, &mut writer(&connect()?)?, args),
        Command::Sync(args) => {
            let client = connect()?;
            cmd::sync::run(
                out,
                term,
                &client,
                &mut catalog(&client),
                github::API,
                &token,
                args,
            )
        }
        Command::History(args) => {
            cmd::history::run(out, term, &connect()?, github::API, &token, args)
        }
        Command::Top(args) => cmd::top::run(out, term, &connect()?, args),
        Command::Version(args) if args.client_only() => cmd::version::run(out, term, None, args),
        Command::Version(args) => cmd::version::run(out, term, Some(&connect()?), args),
        Command::Validate(args) => cmd::validate::run(out, term, args),
        Command::Get(args) => {
            let client = connect()?;
            cmd::get::run(out, term, &client, &mut catalog(&client), args)
        }
        Command::Describe(args) => {
            let client = connect()?;
            cmd::describe::run(out, term, &client, &mut catalog(&client), args)
        }
        Command::Explain(args) => {
            let client = connect()?;
            cmd::explain::run(out, term, &mut catalog(&client), args)
        }
        Command::ApiResources(args) => {
            let client = connect()?;
            cmd::api_resources::run(out, term, &mut catalog(&client), args)
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

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
        ColorArgs::command().debug_assert();
    }

    #[test]
    fn prescan_finds_the_color_flags_anywhere() {
        let scan = |args: &[&str]| ColorArgs::prescan(args.iter().map(OsString::from));
        assert_eq!(
            scan(&[
                "hldr",
                "get",
                "--plain",
                "p",
                "--color-preset",
                "light",
                "--force-colors=256",
                "--help",
            ]),
            ColorArgs {
                plain: true,
                force_colors: Some("256".to_owned()),
                light_background: false,
                color_preset: Some("light".to_owned()),
            }
        );
        assert_eq!(
            scan(&[
                "hldr",
                "--force-colors",
                "--color-preset=none",
                "--light-background"
            ]),
            ColorArgs {
                plain: false,
                force_colors: Some("auto".to_owned()),
                light_background: true,
                color_preset: Some("none".to_owned()),
            }
        );
        assert!(!scan(&["hldr", "get", "--", "--plain"]).plain);
        assert_eq!(scan(&["hldr", "--color-preset"]), ColorArgs::default());
    }
}
