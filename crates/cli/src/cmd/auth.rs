//! `hldr auth`: log in to GitHub, see what the stored credential reaches,
//! and log out. Writes need it; reads work without.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};

use crate::auth::{self, Credential, DeviceCode, OAuth, Store};
use crate::client::Client;
use crate::color::{Painter, Role, Term};

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Action,
}

#[derive(Debug, clap::Subcommand)]
enum Action {
    /// Log in to GitHub through the browser, or with a token
    Login(LoginArgs),
    /// Show the GitHub credential in use and what it reaches
    Status,
    /// Forget the stored GitHub credential
    Logout,
}

#[derive(Debug, clap::Args)]
struct LoginArgs {
    /// Read a token from stdin instead, such as a fine-grained personal
    /// access token with Contents write access to the content repository
    #[arg(long)]
    with_token: bool,
    /// Print where to enter the code instead of opening a browser
    #[arg(long, conflicts_with = "with_token")]
    no_browser: bool,
}

/// What the commands need from the process around them.
pub struct Context<'a> {
    pub oauth: &'a OAuth,
    pub store: Option<&'a Store>,
    /// `HLDR_GITHUB_TOKEN`, which wins over the stored credential.
    pub env_token: Option<&'a str>,
    pub browser: bool,
    pub stdin_terminal: bool,
}

pub fn run(
    out: &mut dyn Write,
    term: &Term,
    context: &Context<'_>,
    connect: &dyn Fn() -> Result<Client>,
    args: &Args,
) -> Result<bool> {
    match &args.command {
        Action::Login(login) => {
            let store = context.store.context(auth::NO_STORE)?;
            if login.with_token {
                login_with_token(out, term, context, store)
            } else {
                login_with_browser(out, term, context, store, login.no_browser)
            }
        }
        Action::Status => status(out, term, context, connect),
        Action::Logout => logout(out, term, context),
    }
}

fn login_with_browser(
    out: &mut dyn Write,
    term: &Term,
    context: &Context<'_>,
    store: &Store,
    no_browser: bool,
) -> Result<bool> {
    let paint = term.out();
    let mut show = |code: &DeviceCode| -> Result<()> {
        writeln!(
            out,
            "First copy your one-time code: {}",
            paint.paint(Role::BasePrimary, &code.user_code)
        )?;
        let url = paint.paint(Role::BaseInfo, &code.verification_uri);
        if context.browser && !no_browser && open(&code.verification_uri) {
            writeln!(out, "Then enter it at {url}, now open in your browser.")?;
        } else {
            writeln!(out, "Then open {url} in a browser and enter it.")?;
        }
        writeln!(out, "Waiting for GitHub...")?;
        out.flush()?;
        Ok(())
    };
    let credential = auth::login(
        store,
        context.oauth,
        &mut show,
        &hldr_core::github::now,
        &std::thread::sleep,
    )?;
    logged_in(out, term, context, &credential)?;
    writeln!(
        out,
        "The session renews itself, and ends once hldr goes unused for six months."
    )?;
    if let Credential::App { access_token, .. } = &credential {
        match context.oauth.repositories(access_token)? {
            Some(repositories) if repositories.is_empty() => term.warning(&format!(
                "the hldr app is installed on no repository; install it on the content \
                 repository at {}",
                auth::INSTALLATIONS
            )),
            Some(repositories) => writeln!(
                out,
                "It reaches {}.",
                paint.paint(Role::DataString, &repositories.join(", "))
            )?,
            None => {}
        }
    }
    Ok(true)
}

fn login_with_token(
    out: &mut dyn Write,
    term: &Term,
    context: &Context<'_>,
    store: &Store,
) -> Result<bool> {
    let token = if context.stdin_terminal {
        read_hidden(term, "Paste the token (it will not show): ")?
    } else {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("reading the token from stdin")?;
        text
    };
    let credential = auth::login_with_token(store, context.oauth, &token)?;
    logged_in(out, term, context, &credential)?;
    if let Credential::Token { expires_at, .. } = credential {
        match expires_at {
            Some(at) => writeln!(
                out,
                "The token expires in {}.",
                auth::span(at.saturating_sub(hldr_core::github::now()))
            )?,
            None => writeln!(out, "The token does not expire.")?,
        }
    }
    Ok(true)
}

fn logged_in(
    out: &mut dyn Write,
    term: &Term,
    context: &Context<'_>,
    credential: &Credential,
) -> Result<()> {
    let paint = term.out();
    writeln!(
        out,
        "{} as {}.",
        paint.paint(Role::StatusSuccess, "Logged in to GitHub"),
        paint.paint(Role::DataString, credential.user())
    )?;
    if context.env_token.is_some() {
        term.warning("HLDR_GITHUB_TOKEN is set, and wins over the stored credential until unset");
    }
    Ok(())
}

/// Reads a line from the terminal with echo off, so the token never shows.
/// Refuses when echo cannot be turned off, rather than showing it.
fn read_hidden(term: &Term, question: &str) -> Result<String> {
    let stty = |arg: &str| {
        Command::new("stty")
            .arg(arg)
            .stdin(Stdio::inherit())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    if !stty("-echo") {
        bail!(
            "cannot hide what is typed here; pipe the token instead, such as \
             `op read op://... | hldr auth login --with-token`"
        );
    }
    term.ask(question);
    let mut line = String::new();
    let read = std::io::stdin().read_line(&mut line);
    stty("echo");
    eprintln!();
    read.context("reading the token")?;
    Ok(line)
}

/// Opens `url`, which [`OAuth::start`] checked is https, in the desktop's
/// browser.
fn open(url: &str) -> bool {
    Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn status(
    out: &mut dyn Write,
    term: &Term,
    context: &Context<'_>,
    connect: &dyn Fn() -> Result<Client>,
) -> Result<bool> {
    let paint = term.out();
    let now = hldr_core::github::now();
    let mut rows: Vec<(&str, String)> = Vec::new();
    let token = if let Some(token) = context.env_token {
        let (user, _) = context.oauth.user(token)?;
        rows.push((
            "Method",
            "HLDR_GITHUB_TOKEN, over any stored credential".to_owned(),
        ));
        rows.push(("User", paint.paint(Role::DataString, &user)));
        None
    } else {
        let store = context.store.context(auth::NO_STORE)?;
        if store.load()?.is_none() {
            writeln!(
                out,
                "{}: run `hldr auth login`",
                paint.paint(Role::StatusError, "Not logged in to GitHub")
            )?;
            return Ok(false);
        }
        // Renews first when due, so what shows is what a write would use.
        let token = auth::token(None, Some(store), context.oauth, term, now)?;
        let credential = store
            .load()?
            .context("the credential went away while it was read")?;
        credential_rows(paint, &credential, now, &mut rows);
        rows.push((
            "Stored in",
            paint.value(&store.path().display().to_string()),
        ));
        token.filter(|_| matches!(credential, Credential::App { .. }))
    };

    let reaches = match &token {
        Some(token) => context.oauth.repositories(token)?,
        None => None,
    };
    if let Some(repositories) = &reaches {
        let listed = if repositories.is_empty() {
            "<none>".to_owned()
        } else {
            repositories.join(", ")
        };
        rows.push(("Reaches", paint.value(&listed)));
    }
    if let Some(row) = content_row(paint, connect, reaches.as_deref()) {
        rows.push(("Content", row));
    }

    let width = rows.iter().map(|(key, _)| key.len()).max().unwrap_or(0) + 1;
    for (key, value) in rows {
        let label = format!("{key}:");
        writeln!(
            out,
            "{}{} {value}",
            paint.nth(Role::DescribeKey, 0, &label),
            " ".repeat(width - label.len())
        )?;
    }
    Ok(true)
}

fn credential_rows(
    paint: Painter<'_>,
    credential: &Credential,
    now: u64,
    rows: &mut Vec<(&str, String)>,
) {
    let until = |at: Option<u64>| match at {
        Some(at) if at <= now => paint.paint(Role::StatusError, "expired"),
        Some(at) => format!("in {}", auth::span(at - now)),
        None => "never".to_owned(),
    };
    match credential {
        Credential::App {
            user,
            expires_at,
            refresh_expires_at,
            ..
        } => {
            rows.push((
                "Method",
                "the hldr GitHub App, through the browser".to_owned(),
            ));
            rows.push(("User", paint.paint(Role::DataString, user)));
            rows.push((
                "Token",
                format!(
                    "renewed an hour ahead; this one expires {}",
                    until(*expires_at)
                ),
            ));
            rows.push((
                "Session",
                format!(
                    "ends {} unless hldr is used before then",
                    until(*refresh_expires_at)
                ),
            ));
        }
        Credential::Token {
            user, expires_at, ..
        } => {
            rows.push(("Method", "a token pasted in".to_owned()));
            rows.push(("User", paint.paint(Role::DataString, user)));
            let expiry = until(*expires_at);
            let soon = expires_at.is_some_and(|at| at > now && at - now < 14 * 86_400);
            let expiry = if soon {
                paint.paint(Role::StatusWarning, &expiry)
            } else {
                expiry
            };
            rows.push(("Expires", expiry));
        }
    }
}

/// The repository the server reads, and whether the app reaches it; `None`
/// with no server configured.
fn content_row(
    paint: Painter<'_>,
    connect: &dyn Fn() -> Result<Client>,
    reaches: Option<&[String]>,
) -> Option<String> {
    let client = connect().ok()?;
    let sync = match client.get("/api/v1/sync") {
        Ok(sync) => sync,
        Err(err) => {
            return Some(format!(
                "{} ({err:#})",
                paint.paint(Role::DataNull, "<unknown>")
            ));
        }
    };
    let Some(repository) = sync["repository"].as_str().map(str::to_owned) else {
        return Some(format!(
            "{} (the server reads a directory)",
            paint.value(sync["source"].as_str().unwrap_or("<unknown>"))
        ));
    };
    let verdict = match reaches {
        Some(reaches) if reaches.contains(&repository) => {
            paint.paint(Role::StatusSuccess, "reached")
        }
        Some(_) => paint.paint(
            Role::StatusError,
            &format!(
                "not reached: install the app on it at {}",
                auth::INSTALLATIONS
            ),
        ),
        None => return Some(paint.paint(Role::DataString, &repository)),
    };
    Some(format!(
        "{} ({verdict})",
        paint.paint(Role::DataString, &repository)
    ))
}

fn logout(out: &mut dyn Write, term: &Term, context: &Context<'_>) -> Result<bool> {
    let paint = term.out();
    let store = context.store.context(auth::NO_STORE)?;
    // A file that does not parse, or is open to others, is removed all the same.
    let method = store
        .load()
        .ok()
        .flatten()
        .map(|credential| match credential {
            Credential::App { .. } => auth::APP_AUTHORIZATIONS,
            Credential::Token { .. } => auth::TOKENS,
        });
    if !store.remove()? {
        writeln!(out, "Not logged in to GitHub.")?;
        return Ok(true);
    }
    writeln!(
        out,
        "{}: removed {}.",
        paint.paint(Role::StatusSuccess, "Logged out of GitHub"),
        paint.value(&store.path().display().to_string())
    )?;
    let revoke = method.unwrap_or(auth::APP_AUTHORIZATIONS);
    writeln!(
        out,
        "GitHub still honors it until it expires; revoke it at {}",
        paint.paint(Role::BaseInfo, revoke)
    )?;
    if context.env_token.is_some() {
        term.warning("HLDR_GITHUB_TOKEN is still set, and writes will use it");
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        args: Args,
    }

    fn run_auth(store: &Store, line: &str) -> (bool, String) {
        let oauth = OAuth::new("http://127.0.0.1:1", "http://127.0.0.1:1", "client");
        let context = Context {
            oauth: &oauth,
            store: Some(store),
            env_token: None,
            browser: false,
            stdin_terminal: false,
        };
        let cli = Cli::parse_from(std::iter::once("auth").chain(line.split(' ')));
        let mut out = Vec::new();
        let no_server = || -> Result<Client> { bail!("no server") };
        let ok = run(&mut out, &Term::plain(), &context, &no_server, &cli.args).unwrap();
        (ok, String::from_utf8(out).unwrap())
    }

    #[test]
    fn status_describes_the_credential_and_logout_forgets_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());

        let (ok, out) = run_auth(&store, "status");
        assert!(!ok, "not logged in fails, as gh auth status does");
        assert_eq!(out, "Not logged in to GitHub: run `hldr auth login`\n");

        let in_three_days = hldr_core::github::now() + 3 * 86_400 + 600;
        store
            .store(&Credential::Token {
                user: "hvpaiva".to_owned(),
                token: "pat".to_owned(),
                expires_at: Some(in_three_days),
            })
            .unwrap();
        let (ok, out) = run_auth(&store, "status");
        assert!(ok);
        assert_eq!(
            out,
            format!(
                "Method:    a token pasted in\n\
                 User:      hvpaiva\n\
                 Expires:   in 3 days\n\
                 Stored in: {}\n",
                store.path().display()
            )
        );

        let (ok, out) = run_auth(&store, "logout");
        assert!(ok);
        assert!(out.starts_with("Logged out of GitHub: removed "), "{out}");
        assert!(out.contains(auth::TOKENS), "{out}");
        assert!(store.load().unwrap().is_none());
        assert_eq!(run_auth(&store, "logout").1, "Not logged in to GitHub.\n");
    }
}
