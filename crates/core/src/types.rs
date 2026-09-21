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
