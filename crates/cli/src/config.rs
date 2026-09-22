//! Where the CLI finds its server: the config file, the environment and the
//! `--server` flag, in rising precedence.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::color::{ColorConfig, ColorEnv};

/// `$XDG_CONFIG_HOME/hldr/config.yaml`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Base URL of the private API, such as `https://apollo.<tailnet>.ts.net:8443`.
    pub server: Option<String>,
    /// The color preset and theme, as kubecolor's `color.yaml` sets them.
    pub color: Option<ColorConfig>,
    /// The pager's command line, over `PAGER`, such as `less -RF`.
    pub pager: Option<String>,
    /// `auto` pages output on a terminal; `never`, the default, does not.
    pub paging: Option<String>,
}

/// The process environment the CLI reads, captured once so the rest of the
/// code never touches global state. No `Debug`: it may carry a token.
#[derive(Default, Clone)]
pub struct Env {
    /// `HLDR_CONFIG`: a config file other than the default.
    pub config: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub xdg_cache_home: Option<PathBuf>,
    pub xdg_state_home: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// `HLDR_GITHUB_TOKEN`, which wins over the stored credential.
    pub github_token: Option<String>,
    /// Whether a browser can open here: a graphical session (`DISPLAY` or
    /// `WAYLAND_DISPLAY`) that is not reached over SSH.
    pub browser: bool,
    /// `HLDR_EDITOR`, `VISUAL` or `EDITOR`, in that order.
    pub editor: Option<String>,
    pub color: ColorEnv,
    /// `PAGER`.
    pub pager: Option<String>,
    /// `PATH`, to find a pager when none is named.
    pub path: Option<OsString>,
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
            xdg_state_home: path("XDG_STATE_HOME").filter(|p| p.is_absolute()),
            home: path("HOME"),
            github_token: std::env::var("HLDR_GITHUB_TOKEN")
                .ok()
                .map(|token| token.trim().to_owned())
                .filter(|token| !token.is_empty()),
            browser: (path("DISPLAY").is_some() || path("WAYLAND_DISPLAY").is_some())
                && path("SSH_CONNECTION").is_none(),
            editor: ["HLDR_EDITOR", "VISUAL", "EDITOR"]
                .iter()
                .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty())),
            color: ColorEnv::from_process(),
            pager: std::env::var("PAGER").ok(),
            path: std::env::var_os("PATH"),
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

    /// `$XDG_STATE_HOME/hldr`: state that belongs to this machine and is
    /// neither configuration nor cache, such as the GitHub credential.
    /// Unlike `~/.config`, it is rarely kept in a dotfiles repository.
    pub fn state_dir(&self) -> Option<PathBuf> {
        let base = self
            .xdg_state_home
            .clone()
            .or_else(|| self.home.as_ref().map(|home| home.join(".local/state")))?;
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
        assert_eq!(
            env("/home/u").state_dir().unwrap(),
            PathBuf::from("/home/u/.local/state/hldr")
        );
        let state = Env {
            xdg_state_home: Some(PathBuf::from("/s")),
            ..env("/home/u")
        };
        assert_eq!(state.state_dir().unwrap(), PathBuf::from("/s/hldr"));
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

        std::fs::write(
            &path,
            "color:\n  preset: protanopia\n  theme:\n    base:\n      danger: fg=white:bg=red\n    table:\n      columns: [cyan, green]\n",
        )
        .unwrap();
        let color = Config::load(Some(&path)).unwrap().color.unwrap();
        assert_eq!(color.preset.as_deref(), Some("protanopia"));

        std::fs::write(&path, "pager: less -R\npaging: auto\n").unwrap();
        let config = Config::load(Some(&path)).unwrap();
        assert_eq!(config.pager.as_deref(), Some("less -R"));
        assert_eq!(config.paging.as_deref(), Some("auto"));
        assert_eq!(color.theme.unwrap()["table"]["columns"][1], "green");

        std::fs::write(&path, "sever: https://api.example.test\n").unwrap();
        let err = Config::load(Some(&path)).unwrap_err();
        assert!(format!("{err:#}").contains("config.yaml"), "{err:#}");
    }

    #[test]
    fn flag_wins_over_config() {
        let config = Config {
            server: Some("https://from-file.test".to_owned()),
            ..Config::default()
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
    fn server_must_be_http() {
        let none = Config::default();
        let err = server(None, &none, Some(Path::new("/c.yaml"))).unwrap_err();
        assert!(err.to_string().contains("/c.yaml"), "{err}");
        for bad in ["apollo:8443", "ftp://x", "https://", "https://x/?a=1"] {
            assert!(server(Some(bad), &none, None).is_err(), "{bad}");
        }
    }
}
