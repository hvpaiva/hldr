//! Colored output in kubecolor's manner: its presets, its theme keys and
//! its color syntax, set in the config file, the environment or by flag.
//! kubecolor re-reads kubectl's text; hldr owns its printers, so they color
//! what they know as they write it, and write the same bytes uncolored.

mod highlight;
mod style;
mod theme;

use anyhow::{Result, bail};

pub use highlight::{json, yaml};
pub use style::{Level, Style};
pub use theme::Role;
use theme::{Background, Overrides, Preset, Theme};

/// The flags and environment that decide whether and how to color.
#[derive(Debug, Default)]
pub struct Options {
    /// `--plain`.
    pub plain: bool,
    /// `--force-colors[=LEVEL]` or `HLDR_FORCE_COLORS`.
    pub force: Option<String>,
    /// `--light-background` or `HLDR_LIGHT_BACKGROUND`.
    pub light_background: bool,
    /// `--color-preset` or `HLDR_COLOR_PRESET`, over `color.preset`.
    pub preset: Option<String>,
}

/// The parts of the process environment colors depend on.
#[derive(Debug, Default, Clone)]
pub struct ColorEnv {
    pub no_color: bool,
    pub force_color: bool,
    pub term: Option<String>,
    pub colorterm: Option<String>,
    /// `fg;bg` as some terminals export it, the one hint of the background
    /// that needs no terminal query.
    pub colorfgbg: Option<String>,
    /// `HLDR_COLOR_THEME_*`, the prefix removed.
    pub theme: Vec<(String, String)>,
}

impl ColorEnv {
    pub fn from_process() -> Self {
        let var = |name| std::env::var(name).ok().filter(|v| !v.is_empty());
        Self {
            no_color: var("NO_COLOR").is_some(),
            force_color: var("FORCE_COLOR").is_some(),
            term: var("TERM"),
            colorterm: var("COLORTERM"),
            colorfgbg: var("COLORFGBG"),
            theme: std::env::vars_os()
                .filter_map(|(name, value)| {
                    let name = name.to_str()?.strip_prefix("HLDR_COLOR_THEME_")?.to_owned();
                    Some((name, value.into_string().ok()?))
                })
                .collect(),
        }
    }
}

/// `color:` in the config file.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorConfig {
    pub preset: Option<String>,
    /// Styles by theme key, nested as kubecolor's `theme:` is.
    pub theme: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Force {
    Never,
    Auto,
    Level(Level),
}

fn force(value: &str) -> Result<Option<Force>> {
    Ok(Some(match value.trim().to_ascii_lowercase().as_str() {
        "" | "false" => return Ok(None),
        "none" => Force::Never,
        "auto" | "true" => Force::Auto,
        "basic" | "3bit" | "3-bit" | "4bit" | "4-bit" => Force::Level(Level::Basic),
        "256" | "8bit" | "8-bit" => Force::Level(Level::Ansi256),
        "truecolor" | "true-color" | "24bit" | "24-bit" => Force::Level(Level::TrueColor),
        _ => bail!("invalid color level {value:?}: use none, auto, basic, 256 or truecolor"),
    }))
}

/// The theme, and how many colors each output stream takes; `None` for a
/// stream that is not colored.
#[derive(Debug)]
pub struct Term {
    theme: Theme,
    out: Option<Level>,
    err: Option<Level>,
}

impl Term {
    /// No colors on either stream.
    pub fn plain() -> Self {
        Self {
            theme: Theme::plain(),
            out: None,
            err: None,
        }
    }

    /// The dark preset in the 16 basic colors on both streams, for tests
    /// that check escapes.
    #[cfg(test)]
    pub fn basic() -> Self {
        let options = Options {
            force: Some("basic".to_owned()),
            preset: Some("dark".to_owned()),
            ..Options::default()
        };
        Self::new(&options, &ColorEnv::default(), None, false, false)
            .expect("the dark preset parses")
    }

    /// Each stream is colored on its own terms, so `2>log` keeps escapes out
    /// of the log while the table on the terminal is still colored.
    pub fn new(
        options: &Options,
        env: &ColorEnv,
        config: Option<&ColorConfig>,
        stdout_tty: bool,
        stderr_tty: bool,
    ) -> Result<Self> {
        let force = options.force.as_deref().map(force).transpose()?.flatten();
        let mut preset: Preset = match options
            .preset
            .as_deref()
            .or(config.and_then(|c| c.preset.as_deref()))
        {
            Some(name) => name.parse()?,
            None => Preset::default(),
        };
        if options.light_background {
            preset.background = Background::Light;
        }
        let dark = match preset.background {
            Background::Dark => true,
            Background::Light => false,
            Background::Auto => !light_from_colorfgbg(env.colorfgbg.as_deref()),
        };
        let mut overrides = match config.and_then(|c| c.theme.as_ref()) {
            Some(theme) => Overrides::from_config(theme)?,
            None => Overrides::default(),
        };
        overrides.extend(Overrides::from_env(&env.theme));
        // Validated even when nothing is colored, so a typo shows up at once.
        let theme = Theme::new(preset, dark, &overrides)?;
        let level = |tty| level(options.plain, force, env, tty);
        Ok(Self {
            theme,
            out: level(stdout_tty),
            err: level(stderr_tty),
        })
    }

    pub fn out(&self) -> Painter<'_> {
        Painter {
            theme: &self.theme,
            level: self.out,
        }
    }

    pub fn err(&self) -> Painter<'_> {
        Painter {
            theme: &self.theme,
            level: self.err,
        }
    }

    /// `error: ...` on stderr, as kubectl words it.
    pub fn error(&self, message: &str) {
        eprintln!(
            "{}",
            self.err()
                .paint(Role::StderrError, &format!("error: {message}"))
        );
    }

    pub fn warning(&self, message: &str) {
        eprintln!(
            "{}",
            self.err()
                .paint(Role::StderrWarning, &format!("warning: {message}"))
        );
    }

    /// Has clap draw help, on stdout, and usage errors, on stderr, in the
    /// theme. clap takes one color choice for both streams, so when only one
    /// of them is colored it decides per stream itself, as it does by
    /// default.
    pub fn help(&self, command: clap::Command) -> clap::Command {
        let choice = match (self.out, self.err) {
            (Some(_), Some(_)) => clap::ColorChoice::Always,
            (None, None) => clap::ColorChoice::Never,
            _ => clap::ColorChoice::Auto,
        };
        let level = self.out.or(self.err).unwrap_or(Level::TrueColor);
        let style = |role| self.theme.style(role).clap(level);
        let styles = clap::builder::Styles::plain()
            .header(style(Role::HelpHeader))
            .usage(style(Role::HelpHeader))
            .literal(style(Role::HelpFlag))
            .placeholder(style(Role::HelpPlaceholder))
            .error(style(Role::StderrError))
            .valid(style(Role::StatusSuccess))
            .invalid(style(Role::StatusWarning));
        command.color(choice).styles(styles)
    }
}

/// Same order as kubecolor: `--plain`, `none`, `NO_COLOR`, then a terminal
/// or a request to color anyway.
fn level(plain: bool, force: Option<Force>, env: &ColorEnv, tty: bool) -> Option<Level> {
    if plain || force == Some(Force::Never) || env.no_color {
        return None;
    }
    let forced = force.is_some() || env.force_color;
    if !tty && !forced {
        return None;
    }
    if let Some(Force::Level(level)) = force {
        return Some(level);
    }
    let colorterm = env.colorterm.as_deref().unwrap_or_default();
    let term = env.term.as_deref().unwrap_or_default();
    if matches!(colorterm, "truecolor" | "24bit") {
        Some(Level::TrueColor)
    } else if term.contains("256color") {
        Some(Level::Ansi256)
    } else if term == "dumb" && !forced {
        None
    } else {
        Some(Level::Basic)
    }
}

/// `COLORFGBG=0;15` puts a light background (7 or 15) last.
fn light_from_colorfgbg(value: Option<&str>) -> bool {
    value
        .and_then(|v| v.rsplit(';').next())
        .and_then(|bg| bg.trim().parse::<u8>().ok())
        .is_some_and(|bg| bg == 7 || bg == 15)
}

/// Colors text for one stream, or leaves it as it is.
#[derive(Clone, Copy)]
pub struct Painter<'a> {
    theme: &'a Theme,
    level: Option<Level>,
}

impl Painter<'_> {
    pub fn paint(&self, key: Role, text: &str) -> String {
        self.with(self.theme.style(key), text)
    }

    /// With the `index`th style of a list key, such as a table's columns.
    pub fn nth(&self, key: Role, index: usize, text: &str) -> String {
        self.with(self.theme.nth(key, index), text)
    }

    pub fn with(&self, style: &Style, text: &str) -> String {
        match self.level {
            Some(level) if !style.is_plain() && !text.is_empty() => {
                format!("\x1b[{}m{text}\x1b[0m", style.sgr(level))
            }
            _ => text.to_owned(),
        }
    }

    /// The color of a value by what it reads as, as kubecolor picks it.
    pub fn value(&self, text: &str) -> String {
        self.paint(value_role(text), text)
    }
}

pub fn value_role(text: &str) -> Role {
    match text {
        "null" | "~" | "<none>" | "<empty>" | "<unknown>" | "<unset>" | "<nil>" | "<invalid>" => {
            Role::DataNull
        }
        "true" | "True" => Role::DataTrue,
        "false" | "False" => Role::DataFalse,
        _ if is_number(text) => Role::DataNumber,
        _ => Role::DataString,
    }
}

fn is_number(text: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    match unsigned.split_once('.') {
        Some((whole, fraction)) => digits(whole) && digits(fraction),
        None => digits(unsigned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(term: &str) -> ColorEnv {
        ColorEnv {
            term: Some(term.to_owned()),
            ..ColorEnv::default()
        }
    }

    #[test]
    fn colors_terminals_unless_told_otherwise() {
        let e = env("xterm-256color");
        assert_eq!(level(false, None, &e, true), Some(Level::Ansi256));
        assert_eq!(level(false, None, &e, false), None);
        assert_eq!(level(true, Some(Force::Auto), &e, true), None);
        assert_eq!(level(false, Some(Force::Never), &e, true), None);
        assert_eq!(
            level(false, Some(Force::Auto), &e, false),
            Some(Level::Ansi256)
        );
        assert_eq!(
            level(false, Some(Force::Level(Level::Basic)), &e, false),
            Some(Level::Basic)
        );
        let no_color = ColorEnv {
            no_color: true,
            ..e.clone()
        };
        assert_eq!(level(false, Some(Force::Auto), &no_color, true), None);
        let forced = ColorEnv {
            force_color: true,
            ..e
        };
        assert_eq!(level(false, None, &forced, false), Some(Level::Ansi256));
        let truecolor = ColorEnv {
            colorterm: Some("truecolor".to_owned()),
            ..env("xterm")
        };
        assert_eq!(level(false, None, &truecolor, true), Some(Level::TrueColor));
        assert_eq!(level(false, None, &env("dumb"), true), None);
        assert_eq!(level(false, None, &env("xterm"), true), Some(Level::Basic));
    }

    #[test]
    fn parses_force_levels() {
        assert_eq!(force("").unwrap(), None);
        assert_eq!(force("true").unwrap(), Some(Force::Auto));
        assert_eq!(
            force("24bit").unwrap(),
            Some(Force::Level(Level::TrueColor))
        );
        assert!(force("rainbow").is_err());
    }

    #[test]
    fn reads_the_background_from_colorfgbg() {
        assert!(light_from_colorfgbg(Some("0;15")));
        assert!(light_from_colorfgbg(Some("0;default;7")));
        assert!(!light_from_colorfgbg(Some("15;0")));
        assert!(!light_from_colorfgbg(None));
    }

    #[test]
    fn plain_streams_write_the_text_as_is() {
        let term = Term::new(&Options::default(), &env("xterm"), None, true, false).unwrap();
        assert_eq!(term.out().paint(Role::DiffAdded, "+a"), "\x1b[32m+a\x1b[0m");
        assert_eq!(term.err().paint(Role::DiffAdded, "+a"), "+a");
        assert_eq!(term.out().paint(Role::DiffAdded, ""), "");
    }

    #[test]
    fn picks_the_preset_and_background() {
        let options = |preset: Option<&str>, light| Options {
            preset: preset.map(str::to_owned),
            light_background: light,
            force: Some("basic".to_owned()),
            ..Options::default()
        };
        let config = ColorConfig {
            preset: Some("none".to_owned()),
            theme: None,
        };
        let key = |term: &Term| term.out().paint(Role::BaseSecondary, "x");
        let from_config = Term::new(
            &options(None, false),
            &ColorEnv::default(),
            Some(&config),
            false,
            false,
        )
        .unwrap();
        assert_eq!(key(&from_config), "x");
        let flag = Term::new(
            &options(Some("dark"), false),
            &ColorEnv::default(),
            Some(&config),
            false,
            false,
        )
        .unwrap();
        assert_eq!(key(&flag), "\x1b[36mx\x1b[0m");
        let light = Term::new(
            &options(Some("dark"), true),
            &ColorEnv::default(),
            None,
            false,
            false,
        )
        .unwrap();
        assert_eq!(key(&light), "\x1b[34mx\x1b[0m");
        let auto_light = ColorEnv {
            colorfgbg: Some("0;15".to_owned()),
            ..ColorEnv::default()
        };
        let auto = Term::new(&options(None, false), &auto_light, None, false, false).unwrap();
        assert_eq!(key(&auto), "\x1b[34mx\x1b[0m");
    }

    #[test]
    fn values_color_by_what_they_read_as() {
        assert_eq!(value_role("<none>"), Role::DataNull);
        assert_eq!(value_role("true"), Role::DataTrue);
        assert_eq!(value_role("-1.5"), Role::DataNumber);
        assert_eq!(value_role("12"), Role::DataNumber);
        assert_eq!(value_role("1."), Role::DataString);
        assert_eq!(value_role("v1"), Role::DataString);
    }
}
