//! Where the CLI finds its server: the config file, the environment and the
//! `--server` flag, in rising precedence.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// `$XDG_CONFIG_HOME/hldr/config.yaml`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Base URL of the private API, such as `https://apollo.<tailnet>.ts.net:8443`.
    pub server: Option<String>,
    pub content: Option<ContentConfig>,
}

/// How to write to the content repository the server reads.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentConfig {
    /// A command that prints a GitHub token on stdout, run only when a
    /// command writes, such as `[op, read, "op://Private/hldr-content/credential"]`.
    pub token_command: Option<Vec<String>>,
}

/// The process environment the CLI reads, captured once so the rest of the
/// code never touches global state. No `Debug`: it may carry a token.
#[derive(Default, Clone)]
pub struct Env {
    /// `HLDR_CONFIG`: a config file other than the default.
    pub config: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub xdg_cache_home: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// `HLDR_GITHUB_TOKEN`, which wins over `content.token_command`.
    pub github_token: Option<String>,
    /// `HLDR_EDITOR`, `VISUAL` or `EDITOR`, in that order.
    pub editor: Option<String>,
}

impl Env {
    pub fn from_process() -> Self {
        let path = |name| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        Self {
            config: path("HLDR_CONFIG"),
            xdg_config_home: path("XDG_CONFIG_HOME").filter(|p| p.is_absolute()),
            xdg_cache_home: path("XDG_CACHE_HOME").filter(|p| p.is_absolute()),
            home: path("HOME"),
            github_token: std::env::var("HLDR_GITHUB_TOKEN")
                .ok()
                .filter(|token| !token.trim().is_empty()),
            editor: ["HLDR_EDITOR", "VISUAL", "EDITOR"]
                .iter()
                .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty())),
        }
    }

    pub fn config_file(&self) -> Option<PathBuf> {
        if let Some(path) = &self.config {
            return Some(path.clone());
        }
        let base = self
            .xdg_config_home
            .clone()
            .or_else(|| self.home.as_ref().map(|home| home.join(".config")))?;
        Some(base.join("hldr/config.yaml"))
    }

    pub fn cache_dir(&self) -> Option<PathBuf> {
        let base = self
            .xdg_cache_home
            .clone()
            .or_else(|| self.home.as_ref().map(|home| home.join(".cache")))?;
        Some(base.join("hldr"))
    }
}

impl Config {
    /// A missing file is an empty config; an unreadable or invalid one is an
    /// error, so a typo never silently points the CLI elsewhere.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
        };
        serde_saphyr::from_str(&text).with_context(|| format!("parse {}", path.display()))
    }
}

/// The GitHub token for writes: `HLDR_GITHUB_TOKEN`, else the output of
/// `content.token_command`. The command runs without a shell, so its
/// arguments are never reinterpreted; its stderr reaches the terminal, where
/// a password manager may ask to unlock.
pub fn github_token(env: &Env, config: &Config) -> Result<Option<String>> {
    if let Some(token) = &env.github_token {
        return Ok(Some(token.trim().to_owned()));
    }
    let Some(command) = config
        .content
        .as_ref()
        .and_then(|content| content.token_command.as_ref())
    else {
        return Ok(None);
    };
    let (program, args) = command
        .split_first()
        .context("content.token_command is empty")?;
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .output()
        .with_context(|| format!("running content.token_command ({program})"))?;
    if !output.status.success() {
        bail!(
            "content.token_command ({program}) failed with {}",
            output.status
        );
    }
    let token = String::from_utf8(output.stdout)
        .context("content.token_command printed something that is not text")?
        .trim()
        .to_owned();
    if token.is_empty() {
        bail!("content.token_command ({program}) printed nothing");
    }
    Ok(Some(token))
}

/// The server to talk to: `--server` (or `HLDR_SERVER`, which clap folds into
/// the flag), then the config file.
pub fn server(flag: Option<&str>, config: &Config, config_file: Option<&Path>) -> Result<String> {
    let Some(raw) = flag.or(config.server.as_deref()) else {
        let file = config_file.map_or_else(
            || "$XDG_CONFIG_HOME/hldr/config.yaml".to_owned(),
            |path| path.display().to_string(),
        );
        bail!("no server configured: pass --server, set HLDR_SERVER, or add `server:` to {file}");
    };
    let url = raw.trim().trim_end_matches('/');
    let Some((scheme, rest)) = url.split_once("://") else {
        bail!("server must be an http or https URL, got {raw:?}");
    };
    if !matches!(scheme, "http" | "https") || rest.is_empty() || rest.contains(['?', '#']) {
        bail!("server must be an http or https URL without query or fragment, got {raw:?}");
    }
    Ok(url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(home: &str) -> Env {
        Env {
            home: Some(PathBuf::from(home)),
            ..Env::default()
        }
    }

    #[test]
    fn config_file_follows_xdg() {
        assert_eq!(
            env("/home/u").config_file().unwrap(),
            PathBuf::from("/home/u/.config/hldr/config.yaml")
        );
        let xdg = Env {
            xdg_config_home: Some(PathBuf::from("/x")),
            ..env("/home/u")
        };
        assert_eq!(
            xdg.config_file().unwrap(),
            PathBuf::from("/x/hldr/config.yaml")
        );
        let explicit = Env {
            config: Some(PathBuf::from("/etc/hldr.yaml")),
            ..xdg
        };
        assert_eq!(
            explicit.config_file().unwrap(),
            PathBuf::from("/etc/hldr.yaml")
        );
        assert_eq!(
            env("/home/u").cache_dir().unwrap(),
            PathBuf::from("/home/u/.cache/hldr")
        );
        assert!(Env::default().config_file().is_none());
    }

    #[test]
    fn loads_and_rejects_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        assert!(Config::load(Some(&path)).unwrap().server.is_none());

        std::fs::write(&path, "server: https://api.example.test\n").unwrap();
        let config = Config::load(Some(&path)).unwrap();
        assert_eq!(config.server.as_deref(), Some("https://api.example.test"));

        std::fs::write(&path, "sever: https://api.example.test\n").unwrap();
        let err = Config::load(Some(&path)).unwrap_err();
        assert!(format!("{err:#}").contains("config.yaml"), "{err:#}");
    }

    #[test]
    fn flag_wins_over_config() {
        let config = Config {
            server: Some("https://from-file.test".to_owned()),
            content: None,
        };
        assert_eq!(
            server(None, &config, None).unwrap(),
            "https://from-file.test"
        );
        assert_eq!(
            server(Some("http://127.0.0.1:8081/"), &config, None).unwrap(),
            "http://127.0.0.1:8081"
        );
    }

    #[test]
    fn token_comes_from_env_or_command() {
        let command = |argv: &[&str]| Config {
            server: None,
            content: Some(ContentConfig {
                token_command: Some(argv.iter().map(|a| (*a).to_owned()).collect()),
            }),
        };
        assert_eq!(
            github_token(&Env::default(), &Config::default()).unwrap(),
            None
        );
        assert_eq!(
            github_token(&Env::default(), &command(&["printf", "  tok\\n"]))
                .unwrap()
                .as_deref(),
            Some("tok")
        );
        let env = Env {
            github_token: Some("from-env".to_owned()),
            ..Env::default()
        };
        assert_eq!(
            github_token(&env, &command(&["false"])).unwrap().as_deref(),
            Some("from-env")
        );
        assert!(github_token(&Env::default(), &command(&["false"])).is_err());
        assert!(github_token(&Env::default(), &command(&["true"])).is_err());
        assert!(github_token(&Env::default(), &command(&[])).is_err());
    }

    #[test]
    fn server_must_be_http() {
        let none = Config::default();
        let err = server(None, &none, Some(Path::new("/c.yaml"))).unwrap_err();
        assert!(err.to_string().contains("/c.yaml"), "{err}");
        for bad in ["apollo:8443", "ftp://x", "https://", "https://x/?a=1"] {
            assert!(server(Some(bad), &none, None).is_err(), "{bad}");
        }
    }
}
