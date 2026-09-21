use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Where a project stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStatus {
    /// Maintained and in use.
    Active,
    /// Under construction; may change or break.
    Wip,
    /// No longer maintained; kept for reference.
    Archived,
}

impl ProjectStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Wip => "wip",
            Self::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Result<Self, crate::Error> {
        match s {
            "active" => Ok(Self::Active),
            "wip" => Ok(Self::Wip),
            "archived" => Ok(Self::Archived),
            _ => Err(crate::Error::Invariant("projects.status")),
        }
    }
}

/// External links of a project. Every one is optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectLinks {
    /// Source repository URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Documentation URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    /// Live demo URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demo: Option<String>,
}

impl ProjectLinks {
    pub fn is_empty(&self) -> bool {
        self.repo.is_none() && self.docs.is_none() && self.demo.is_none()
    }
}

/// External links of the profile. Every one is optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileLinks {
    /// GitHub profile URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,
    /// LinkedIn profile URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linkedin: Option<String>,
    /// Repository the profile page links as its source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl ProfileLinks {
    pub fn is_empty(&self) -> bool {
        self.github.is_none() && self.linkedin.is_none() && self.source.is_none()
    }
}

/// An image attached to a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetSpec {
    /// Image path, relative to the assets directory.
    pub path: String,
    /// Text shown under the image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

/// A color as `#rrggbb`. Palettes end up inside inline CSS, so nothing else
/// is accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Color(String);

impl Color {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Color {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let valid = value.len() == 7
            && value.starts_with('#')
            && value[1..].chars().all(|c| c.is_ascii_hexdigit());
        if valid {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err(format!("color must be #rrggbb, got {value:?}"))
        }
    }
}

impl From<Color> for String {
    fn from(color: Color) -> Self {
        color.0
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl JsonSchema for Color {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Color".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^#[0-9a-fA-F]{6}$",
            "description": "A color as #rrggbb."
        })
    }
}

/// The colors a theme paints the site with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    /// Page background.
    pub bg: Color,
    /// Body text.
    pub fg: Color,
    /// Secondary text.
    pub fg_dim: Color,
    /// Comments, rules and other quiet text.
    pub fg_muted: Color,
    /// Links and the main emphasis.
    pub accent: Color,
    /// Second emphasis, such as headings.
    pub accent2: Color,
    /// Current line and selections.
    pub highlight: Color,
    /// Keywords and markers.
    pub special: Color,
    /// Strings and quoted values.
    pub teal: Color,
    /// Errors and warnings.
    pub error: Color,
    /// Borders and separators.
    pub border: Color,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_six_digit_hex() {
        assert_eq!(
            Color::try_from("#FAA968".to_owned()).unwrap().as_str(),
            "#faa968"
        );
        for bad in [
            "faa968", "#faa96", "#faa9688", "#fa a968", "#gggggg", "red", "</style>",
        ] {
            assert!(Color::try_from(bad.to_owned()).is_err(), "{bad}");
        }
    }
}
