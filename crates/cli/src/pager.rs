//! Output through a pager, as kubecolor offers it: opt-in, `$PAGER` or
//! `less -RF` by default, and only when stdout is a terminal.

use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};

use anyhow::{Result, bail};

use crate::color::Term;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paging {
    Auto,
    Never,
}

impl std::str::FromStr for Paging {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "never" => Ok(Self::Never),
            _ => bail!("invalid paging mode {value:?}: use auto or never"),
        }
    }
}

/// Where each setting comes from, in falling precedence.
#[derive(Debug, Default)]
pub struct Settings<'a> {
    /// `--no-paging`.
    pub no_paging: bool,
    /// `--paging[=MODE]` or `HLDR_PAGING`.
    pub paging: Option<&'a str>,
    /// `paging:` in the config file.
    pub config_paging: Option<&'a str>,
    /// `--pager` or `HLDR_PAGER`.
    pub pager: Option<&'a str>,
    /// `pager:` in the config file.
    pub config_pager: Option<&'a str>,
    /// `PAGER`, which the config file overrides, as in kubecolor.
    pub env_pager: Option<&'a str>,
    /// `PATH`, to find `less` or `more` when nothing names a pager.
    pub path: Option<&'a OsStr>,
}

impl Settings<'_> {
    /// The pager's command line, split on whitespace and run without a shell,
    /// or `None` when output goes straight to stdout. Paging is off unless
    /// asked for, and a mode that is not one is an error even then.
    pub fn command(&self) -> Result<Option<Vec<String>>> {
        let mode = match self.paging.or(self.config_paging) {
            Some(mode) => mode.parse()?,
            None => Paging::Never,
        };
        if self.no_paging || mode == Paging::Never {
            return Ok(None);
        }
        let named = [self.pager, self.config_pager, self.env_pager]
            .into_iter()
            .flatten()
            .find(|pager| !pager.trim().is_empty());
        let line = match named {
            Some(line) => line,
            None if on_path(self.path, "less") => "less -RF",
            None if on_path(self.path, "more") => "more",
            None => return Ok(None),
        };
        Ok(Some(line.split_whitespace().map(str::to_owned).collect()))
    }
}

fn on_path(path: Option<&OsStr>, program: &str) -> bool {
    path.is_some_and(|path| {
        std::env::split_paths(path).any(|dir| Path::new(&dir).join(program).is_file())
    })
}

/// Stdout, or a pager's stdin. The pager starts on the first write, so a
/// command that prints nothing never starts one, and whatever runs before
/// the output, such as an editor, still has the terminal.
pub struct Pager<'a> {
    command: Option<Vec<String>>,
    running: Option<(Child, ChildStdin)>,
    term: &'a Term,
}

impl<'a> Pager<'a> {
    pub fn new(command: Option<Vec<String>>, term: &'a Term) -> Self {
        Self {
            command,
            running: None,
            term,
        }
    }

    /// Closes the pager's input and waits for it, so the shell prompt and
    /// any error come after the pager exits.
    pub fn finish(mut self) -> io::Result<()> {
        if let Some((mut child, stdin)) = self.running.take() {
            drop(stdin);
            child.wait()?;
        }
        Ok(())
    }

    /// Starts the pager if it is due; one that fails to start is reported
    /// and output goes to stdout.
    fn start(&mut self) {
        if let Some(argv) = self.command.take() {
            match spawn(&argv) {
                Ok(running) => self.running = Some(running),
                Err(err) => self
                    .term
                    .warning(&format!("pager {:?} did not start: {err}", argv.join(" "))),
            }
        }
    }
}

fn spawn(argv: &[String]) -> io::Result<(Child, ChildStdin)> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::other("the pager command is empty"))?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("the pager has no stdin"))?;
    Ok((child, stdin))
}

impl Write for Pager<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.start();
        match &mut self.running {
            Some((_, stdin)) => stdin.write(buf),
            None => io::stdout().lock().write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.running {
            Some((_, stdin)) => stdin.flush(),
            None if self.command.is_none() => io::stdout().lock().flush(),
            // Nothing was written, so the pager has not started.
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(settings: Settings<'_>) -> Option<String> {
        settings.command().unwrap().map(|argv| argv.join(" "))
    }

    #[test]
    fn paging_is_opt_in() {
        assert_eq!(command(Settings::default()), None);
        let asked = || Settings {
            paging: Some("auto"),
            env_pager: Some("most"),
            ..Settings::default()
        };
        assert_eq!(command(asked()).as_deref(), Some("most"));
        assert_eq!(
            command(Settings {
                no_paging: true,
                ..asked()
            }),
            None
        );
        let from_config = Settings {
            config_paging: Some("auto"),
            env_pager: Some("most"),
            ..Settings::default()
        };
        assert_eq!(command(from_config).as_deref(), Some("most"));
        let flag_wins = Settings {
            paging: Some("never"),
            config_paging: Some("auto"),
            env_pager: Some("most"),
            ..Settings::default()
        };
        assert_eq!(command(flag_wins), None);
        assert!(
            Settings {
                config_paging: Some("sometimes"),
                ..Settings::default()
            }
            .command()
            .is_err()
        );
    }

    #[test]
    fn the_pager_comes_from_flag_config_env_then_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().as_os_str();
        let settings = |pager, config_pager, env_pager| Settings {
            paging: Some("auto"),
            pager,
            config_pager,
            env_pager,
            path: Some(path),
            ..Settings::default()
        };
        assert_eq!(
            command(settings(Some("a -x"), Some("b"), Some("c"))).as_deref(),
            Some("a -x")
        );
        assert_eq!(
            command(settings(None, Some("b"), Some("c"))).as_deref(),
            Some("b")
        );
        assert_eq!(
            command(settings(Some(" "), None, Some("c"))).as_deref(),
            Some("c")
        );
        assert_eq!(command(settings(None, None, None)), None);
        std::fs::write(dir.path().join("more"), "").unwrap();
        assert_eq!(command(settings(None, None, None)).as_deref(), Some("more"));
        std::fs::write(dir.path().join("less"), "").unwrap();
        assert_eq!(
            command(settings(None, None, None)).as_deref(),
            Some("less -RF")
        );
    }

    #[test]
    fn starts_on_the_first_write_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("paged");
        let argv = || {
            vec![
                "dd".to_owned(),
                format!("of={}", file.display()),
                "status=none".to_owned(),
            ]
        };
        let term = Term::plain();

        let mut idle = Pager::new(Some(argv()), &term);
        idle.flush().unwrap();
        idle.finish().unwrap();
        assert!(!file.exists());

        let mut pager = Pager::new(Some(argv()), &term);
        writeln!(pager, "one").unwrap();
        writeln!(pager, "two").unwrap();
        pager.flush().unwrap();
        pager.finish().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\ntwo\n");
    }
}
