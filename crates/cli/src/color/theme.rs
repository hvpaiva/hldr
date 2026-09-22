//! The theme: kubecolor's keys, presets and fallbacks, kept to what hldr
//! prints. A key left unset takes the value of the key it falls back to, so
//! setting `base.danger` recolors every error, removal and deletion at once.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::style::{PLAIN, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    BaseKey,
    BaseInfo,
    BasePrimary,
    BaseSecondary,
    BaseSuccess,
    BaseWarning,
    BaseDanger,
    BaseMuted,
    DataKey,
    DataString,
    DataTrue,
    DataFalse,
    DataNumber,
    DataNull,
    StatusSuccess,
    StatusWarning,
    StatusError,
    TableHeader,
    TableColumns,
    StderrError,
    StderrWarning,
    DescribeKey,
    ExplainKey,
    ExplainRequired,
    DiffAdded,
    DiffRemoved,
    DiffUnchanged,
    ApplyCreated,
    ApplyConfigured,
    ApplyUnchanged,
    ApplyDryRun,
    DeleteDeleted,
    PatchPatched,
    VersionKey,
    SyncCommitted,
    SyncSynced,
    SyncSkipped,
}

struct Spec {
    key: Role,
    name: &'static str,
    /// Takes several styles, cycled by column or by depth.
    list: bool,
    /// What an unset key takes: the one key's value, or with several keys,
    /// a list of their first styles.
    from: &'static [Role],
}

const fn color(key: Role, name: &'static str, from: &'static [Role]) -> Spec {
    Spec {
        key,
        name,
        list: false,
        from,
    }
}

const fn list(key: Role, name: &'static str, from: &'static [Role]) -> Spec {
    Spec {
        key,
        name,
        list: true,
        from,
    }
}

use Role::*;

/// In the order of [`Role`], so a role's spec is `SPECS[role as usize]`.
const SPECS: &[Spec] = &[
    list(BaseKey, "base.key", &[BaseSecondary]),
    color(BaseInfo, "base.info", &[]),
    color(BasePrimary, "base.primary", &[]),
    color(BaseSecondary, "base.secondary", &[]),
    color(BaseSuccess, "base.success", &[]),
    color(BaseWarning, "base.warning", &[]),
    color(BaseDanger, "base.danger", &[]),
    color(BaseMuted, "base.muted", &[]),
    list(DataKey, "data.key", &[BaseKey]),
    color(DataString, "data.string", &[BaseInfo]),
    color(DataTrue, "data.true", &[BaseSuccess]),
    color(DataFalse, "data.false", &[BaseDanger]),
    color(DataNumber, "data.number", &[BasePrimary]),
    color(DataNull, "data.null", &[BaseMuted]),
    color(StatusSuccess, "status.success", &[BaseSuccess]),
    color(StatusWarning, "status.warning", &[BaseWarning]),
    color(StatusError, "status.error", &[BaseDanger]),
    color(TableHeader, "table.header", &[BaseInfo]),
    list(TableColumns, "table.columns", &[BaseInfo, BaseSecondary]),
    color(StderrError, "stderr.error", &[BaseDanger]),
    color(StderrWarning, "stderr.warning", &[BaseWarning]),
    list(DescribeKey, "describe.key", &[BaseKey]),
    list(ExplainKey, "explain.key", &[BaseKey]),
    color(ExplainRequired, "explain.required", &[BaseDanger]),
    color(DiffAdded, "diff.added", &[BaseSuccess]),
    color(DiffRemoved, "diff.removed", &[BaseDanger]),
    color(DiffUnchanged, "diff.unchanged", &[BaseMuted]),
    color(ApplyCreated, "apply.created", &[BaseSuccess]),
    color(ApplyConfigured, "apply.configured", &[BaseWarning]),
    color(ApplyUnchanged, "apply.unchanged", &[BasePrimary]),
    color(ApplyDryRun, "apply.dryrun", &[BaseSecondary]),
    color(DeleteDeleted, "delete.deleted", &[BaseDanger]),
    color(PatchPatched, "patch.patched", &[BaseWarning]),
    list(VersionKey, "version.key", &[BaseKey]),
    // hldr's own: kubectl neither commits nor syncs.
    color(SyncCommitted, "sync.committed", &[BaseSuccess]),
    color(SyncSynced, "sync.synced", &[BaseSuccess]),
    color(SyncSkipped, "sync.skipped", &[BaseWarning]),
];

fn spec(key: Role) -> &'static Spec {
    &SPECS[key as usize]
}

fn by_name(name: &str) -> Result<&'static Spec> {
    SPECS
        .iter()
        .find(|spec| spec.name.eq_ignore_ascii_case(name))
        .with_context(|| {
            let known: Vec<&str> = SPECS.iter().map(|spec| spec.name).collect();
            format!(
                "unknown theme key {name:?}; the keys are {}",
                known.join(", ")
            )
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    Auto,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    /// No colors of its own; only what the config sets.
    None,
    Default,
    Protanopia,
    Deuteranopia,
    Tritanopia,
}

/// A family of colors on a background, named as kubecolor names them:
/// `dark`, `light`, `auto`, `protanopia-dark`, `tritanopia` (auto) and so on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    family: Family,
    pub background: Background,
}

impl Default for Preset {
    fn default() -> Self {
        Self {
            family: Family::Default,
            background: Background::Auto,
        }
    }
}

impl std::str::FromStr for Preset {
    type Err = anyhow::Error;

    fn from_str(name: &str) -> Result<Self> {
        let name = name.trim().to_ascii_lowercase();
        let background = |suffix: &str| match suffix {
            "" => Some(Background::Auto),
            "-dark" => Some(Background::Dark),
            "-light" => Some(Background::Light),
            _ => None,
        };
        let preset = match name.as_str() {
            "none" | "" => Some((Family::None, Background::Auto)),
            "auto" => Some((Family::Default, Background::Auto)),
            "dark" => Some((Family::Default, Background::Dark)),
            "light" => Some((Family::Default, Background::Light)),
            _ => [
                ("protanopia", Family::Protanopia),
                ("deuteranopia", Family::Deuteranopia),
                ("tritanopia", Family::Tritanopia),
            ]
            .into_iter()
            .find_map(|(prefix, family)| Some((family, background(name.strip_prefix(prefix)?)?))),
        };
        match preset {
            Some((family, background)) => Ok(Self { family, background }),
            None => bail!(
                "unknown color preset {name:?}: use auto, dark, light, none, or protanopia, \
                 deuteranopia or tritanopia, each alone or with -dark or -light"
            ),
        }
    }
}

impl Preset {
    /// kubecolor's values, spec for spec.
    fn styles(self, dark: bool) -> Vec<(Role, &'static str)> {
        let (fg, key) = if dark {
            ("white", "hicyan / cyan")
        } else {
            ("black", "hiblue / blue")
        };
        match self.family {
            Family::None => Vec::new(),
            Family::Default => vec![
                (BaseKey, key),
                (BaseInfo, fg),
                (BasePrimary, "magenta"),
                (BaseSecondary, if dark { "cyan" } else { "blue" }),
                (BaseSuccess, "green"),
                (BaseWarning, "yellow"),
                (BaseDanger, "red"),
                (BaseMuted, "gray:italic"),
                (TableHeader, "bold"),
                (DataString, if dark { "hiyellow" } else { "yellow" }),
            ],
            Family::Protanopia | Family::Deuteranopia | Family::Tritanopia => {
                let italic_muted = !dark || self.family == Family::Protanopia;
                vec![
                    (BaseKey, "#feb927 / #fe6e1a"),
                    (BaseInfo, fg),
                    (BasePrimary, "#4860e6"),
                    (BaseSecondary, "#2aabee"),
                    (BaseSuccess, "#6afd6a:bold"),
                    (BaseWarning, "#feb927:italic"),
                    (
                        BaseDanger,
                        if dark {
                            "fg=white:bg=#c2270a"
                        } else {
                            "fg=black:bg=#c2270a"
                        },
                    ),
                    (
                        BaseMuted,
                        if italic_muted {
                            "#2ee5ae:italic"
                        } else {
                            "#2ee5ae"
                        },
                    ),
                    (DataString, "#2aabee"),
                    (TableHeader, if dark { "white:bold" } else { "black:bold" }),
                    (
                        TableColumns,
                        if dark {
                            "#2aabee / #6afd6a:bold / #4860e6 / white / #feb927"
                        } else {
                            "#2aabee / #6afd6a:bold / #4860e6 / black / #feb927"
                        },
                    ),
                ]
            }
        }
    }
}

/// Styles set by name, from the config file or the environment, over the
/// preset's.
#[derive(Debug, Default)]
pub struct Overrides(Vec<(String, Vec<String>)>);

impl Overrides {
    /// `theme:` from the config file: nested maps, each leaf a spec or a
    /// list of specs.
    pub fn from_config(theme: &serde_json::Map<String, Value>) -> Result<Self> {
        fn walk(
            prefix: &str,
            map: &serde_json::Map<String, Value>,
            out: &mut Vec<(String, Vec<String>)>,
        ) -> Result<()> {
            for (key, value) in map {
                let name = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                let scalar = |value: &Value| match value {
                    Value::String(text) => Ok(text.clone()),
                    Value::Number(number) => Ok(number.to_string()),
                    other => bail!("theme.{name}: expected a color, got {other}"),
                };
                match value {
                    Value::Object(inner) => walk(&name, inner, out)?,
                    Value::Array(items) => {
                        out.push((
                            name.clone(),
                            items.iter().map(scalar).collect::<Result<_>>()?,
                        ));
                    }
                    other => out.push((name.clone(), vec![scalar(other)?])),
                }
            }
            Ok(())
        }
        let mut out = Vec::new();
        walk("", theme, &mut out)?;
        Ok(Self(out))
    }

    /// `HLDR_COLOR_THEME_TABLE_COLUMNS=cyan / green` sets `table.columns`.
    pub fn from_env(vars: &[(String, String)]) -> Self {
        Self(
            vars.iter()
                .map(|(name, value)| {
                    (
                        name.to_ascii_lowercase().replace('_', "."),
                        vec![value.clone()],
                    )
                })
                .collect(),
        )
    }

    pub fn extend(&mut self, other: Self) {
        self.0.extend(other.0);
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    styles: Vec<Vec<Style>>,
}

impl Theme {
    #[cfg(test)]
    pub fn plain() -> Self {
        Self {
            styles: vec![Vec::new(); SPECS.len()],
        }
    }

    pub fn new(preset: Preset, dark: bool, overrides: &Overrides) -> Result<Self> {
        let mut set: BTreeMap<Role, Vec<Style>> = BTreeMap::new();
        for (key, text) in preset.styles(dark) {
            set.insert(key, parse(spec(key), &[text.to_owned()])?);
        }
        for (name, texts) in &overrides.0 {
            let spec = by_name(name)?;
            set.insert(spec.key, parse(spec, texts)?);
        }
        let styles = SPECS.iter().map(|spec| resolve(spec, &set)).collect();
        Ok(Self { styles })
    }

    pub fn style(&self, key: Role) -> &Style {
        self.styles[key as usize].first().unwrap_or(&PLAIN)
    }

    /// The `index`th style of a list key, cycling: columns left to right,
    /// or keys by depth.
    pub fn nth(&self, key: Role, index: usize) -> &Style {
        let styles = &self.styles[key as usize];
        if styles.is_empty() {
            return &PLAIN;
        }
        &styles[index % styles.len()]
    }
}

fn parse(spec: &Spec, texts: &[String]) -> Result<Vec<Style>> {
    let mut styles = Vec::new();
    for text in texts {
        let parsed = Style::parse_list(text).with_context(|| format!("theme.{}", spec.name))?;
        styles.extend(parsed);
    }
    if !spec.list && styles.len() > 1 {
        bail!("theme.{} takes one color, not a list", spec.name);
    }
    Ok(styles)
}

fn resolve(spec: &Spec, set: &BTreeMap<Role, Vec<Style>>) -> Vec<Style> {
    if let Some(styles) = set.get(&spec.key) {
        return styles.clone();
    }
    match spec.from {
        [] => Vec::new(),
        [one] => {
            let mut styles = resolve(self::spec(*one), set);
            if !spec.list {
                styles.truncate(1);
            }
            styles
        }
        many => many
            .iter()
            .filter_map(|key| resolve(self::spec(*key), set).into_iter().next())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::style::Level;
    use super::*;

    fn sgr(theme: &Theme, key: Role) -> String {
        theme.style(key).sgr(Level::TrueColor)
    }

    #[test]
    fn specs_follow_the_key_order() {
        for (i, spec) in SPECS.iter().enumerate() {
            assert_eq!(spec.key as usize, i, "{}", spec.name);
        }
    }

    #[test]
    fn every_preset_parses() {
        for name in [
            "none",
            "auto",
            "dark",
            "light",
            "protanopia",
            "protanopia-dark",
            "protanopia-light",
            "deuteranopia",
            "deuteranopia-dark",
            "deuteranopia-light",
            "tritanopia",
            "tritanopia-dark",
            "tritanopia-light",
        ] {
            let preset: Preset = name.parse().unwrap();
            for dark in [true, false] {
                Theme::new(preset, dark, &Overrides::default()).unwrap();
            }
        }
        assert!("solarized".parse::<Preset>().is_err());
        assert!("protanopia-dim".parse::<Preset>().is_err());
    }

    #[test]
    fn unset_keys_fall_back() {
        let dark = Theme::new(Preset::default(), true, &Overrides::default()).unwrap();
        assert_eq!(sgr(&dark, DiffRemoved), "31");
        assert_eq!(sgr(&dark, DataNull), "90;3");
        assert_eq!(sgr(&dark, DataString), "93");
        assert_eq!(dark.nth(DescribeKey, 1).sgr(Level::Basic), "36");
        assert_eq!(dark.nth(TableColumns, 0).sgr(Level::Basic), "37");
        assert_eq!(dark.nth(TableColumns, 3).sgr(Level::Basic), "36");
    }

    #[test]
    fn overrides_win_and_flow_down() {
        let mut overrides = Overrides::from_config(
            serde_json::json!({
                "base": {"danger": "magenta:bold"},
                "table": {"columns": ["cyan", 208]},
            })
            .as_object()
            .unwrap(),
        )
        .unwrap();
        overrides.extend(Overrides::from_env(&[(
            "DIFF_ADDED".to_owned(),
            "hiblue".to_owned(),
        )]));
        let theme = Theme::new(Preset::default(), true, &overrides).unwrap();
        assert_eq!(sgr(&theme, StderrError), "35;1");
        assert_eq!(sgr(&theme, DiffRemoved), "35;1");
        assert_eq!(sgr(&theme, DiffAdded), "94");
        assert_eq!(theme.nth(TableColumns, 1).sgr(Level::TrueColor), "38;5;208");
    }

    #[test]
    fn rejects_unknown_keys_and_lists_for_colors() {
        let bad = |json: Value| {
            Overrides::from_config(json.as_object().unwrap())
                .and_then(|o| Theme::new(Preset::default(), true, &o))
                .unwrap_err()
                .to_string()
        };
        assert!(bad(serde_json::json!({"base": {"dnager": "red"}})).contains("dnager"));
        assert!(bad(serde_json::json!({"diff": {"added": "red / green"}})).contains("one color"));
        assert!(bad(serde_json::json!({"diff": {"added": true}})).contains("diff.added"));
    }
}
