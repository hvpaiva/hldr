//! Content files as the content repository stores them.
//!
//! The indexer and the CLI share this parser, so what `hldr edit` accepts is
//! exactly what the server indexes. Prose kinds are markdown under a YAML
//! frontmatter; configuration kinds are plain YAML. Every file declares its
//! `kind`, and where the file sits must agree with it.

use std::path::{Component, Path};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Error;
use crate::api::{
    BannerSpec, BlogSpec, DescriptionsSpec, ProfileSpec, ProjectSpec, SiteSpec, ThemeSpec,
};
use crate::types::{AssetSpec, Palette, ProfileLinks, ProjectLinks, ProjectStatus};

/// A resource type backed by a content file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Project,
    Profile,
    Site,
    Theme,
}

/// How a content file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// YAML frontmatter between `---` lines, then a markdown body.
    Markdown,
    /// A plain YAML document.
    Yaml,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "Project",
            Self::Profile => "Profile",
            Self::Site => "Site",
            Self::Theme => "Theme",
        }
    }

    /// Path under the content root; `{name}` stands for the resource name.
    pub fn path_template(self) -> &'static str {
        match self {
            Self::Project => "projects/{name}.md",
            Self::Profile => "profile.md",
            Self::Site => "site.yaml",
            Self::Theme => "themes/{name}.yaml",
        }
    }

    pub fn format(self) -> Format {
        match self {
            Self::Project | Self::Profile => Format::Markdown,
            Self::Site | Self::Theme => Format::Yaml,
        }
    }

    /// Exactly one resource of this kind exists, and it has no name.
    pub fn is_singleton(self) -> bool {
        matches!(self, Self::Profile | Self::Site)
    }
}

/// What a content path holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub kind: Kind,
    /// Resource name; `None` for singletons.
    pub name: Option<String>,
}

/// Resolves a path relative to the content root. Files outside the layout,
/// such as a README, hold no resource and resolve to `None`.
pub fn locate(path: &Path) -> Result<Option<Location>, Error> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(
                part.to_str()
                    .ok_or_else(|| Error::file(path, "path is not valid UTF-8"))?,
            ),
            _ => {
                return Err(Error::file(
                    path,
                    "path must be relative to the content root",
                ));
            }
        }
    }
    let location = match parts.as_slice() {
        ["site.yaml"] => Location {
            kind: Kind::Site,
            name: None,
        },
        ["profile.md"] => Location {
            kind: Kind::Profile,
            name: None,
        },
        ["projects", file] => match file.strip_suffix(".md") {
            Some(name) => Location {
                kind: Kind::Project,
                name: Some(validate_name(path, name)?.to_owned()),
            },
            None => return Ok(None),
        },
        ["themes", file] => match file.strip_suffix(".yaml") {
            Some(name) => Location {
                kind: Kind::Theme,
                name: Some(validate_name(path, name)?.to_owned()),
            },
            None => return Ok(None),
        },
        _ => return Ok(None),
    };
    Ok(Some(location))
}

/// Path of a resource under the content root.
pub fn path(kind: Kind, name: Option<&str>) -> String {
    match name {
        Some(name) => kind.path_template().replace("{name}", name),
        None => kind.path_template().to_owned(),
    }
}

/// Whether `name` can name a resource: `[a-z0-9][a-z0-9-]*`. Names end up
/// in file paths and URL paths, so nothing else is accepted.
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| c.is_ascii_lowercase() || c.is_ascii_digit() || (i > 0 && c == '-'))
}

fn validate_name<'a>(path: &Path, name: &'a str) -> Result<&'a str, Error> {
    if is_valid_name(name) {
        Ok(name)
    } else {
        Err(Error::file(path, "name must match [a-z0-9][a-z0-9-]*"))
    }
}

/// A parsed content file.
#[derive(Debug, Clone)]
pub enum Manifest {
    Project { name: String, spec: ProjectSpec },
    Profile(ProfileSpec),
    Site(SiteSpec),
    Theme { name: String, spec: ThemeSpec },
}

impl Manifest {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Project { .. } => Kind::Project,
            Self::Profile(_) => Kind::Profile,
            Self::Site(_) => Kind::Site,
            Self::Theme { .. } => Kind::Theme,
        }
    }

    /// Path of the file under the content root.
    pub fn path(&self) -> String {
        match self {
            Self::Project { name, .. } | Self::Theme { name, .. } => path(self.kind(), Some(name)),
            other => path(other.kind(), None),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectFile {
    kind: String,
    title: String,
    tagline: String,
    status: ProjectStatus,
    #[serde(default, skip_serializing_if = "is_false")]
    draft: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    highlight: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(default, skip_serializing_if = "ProjectLinks::is_empty")]
    links: ProjectLinks,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    github: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    assets: Vec<AssetSpec>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    kind: String,
    name: String,
    headline: String,
    bio: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(default, skip_serializing_if = "ProfileLinks::is_empty")]
    links: ProfileLinks,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SiteFile {
    kind: String,
    title: String,
    theme: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    banner: Option<BannerSpec>,
    descriptions: DescriptionsSpec,
    blog: BlogSpec,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    kind: String,
    title: String,
    dark: bool,
    colors: Palette,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// The `kind` a file declares, read before knowing where the file belongs:
/// from the frontmatter of a markdown file, or from a YAML document.
pub fn declared_kind(source: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Head {
        kind: String,
    }
    let yaml = match split_frontmatter(Path::new(""), source) {
        Ok((frontmatter, _)) => frontmatter,
        Err(_) => source,
    };
    serde_saphyr::from_str::<Head>(yaml)
        .ok()
        .map(|head| head.kind)
}

/// Parses a content file. `path` is relative to the content root and decides
/// which kind the file must declare.
pub fn parse(path: &Path, source: &[u8]) -> Result<Manifest, Error> {
    let location = locate(path)?.ok_or_else(|| Error::file(path, "not a content file"))?;
    let text = std::str::from_utf8(source).map_err(|_| Error::file(path, "not valid UTF-8"))?;
    match (location.kind, location.name) {
        (Kind::Project, Some(name)) => {
            let (frontmatter, body) = split_frontmatter(path, text)?;
            let file: ProjectFile = from_yaml(path, frontmatter)?;
            expect_kind(path, &file.kind, Kind::Project)?;
            let spec = ProjectSpec {
                title: file.title,
                tagline: file.tagline,
                status: file.status,
                draft: file.draft,
                highlight: file.highlight,
                tags: file.tags,
                links: file.links,
                github: file.github,
                assets: file.assets,
                body: body.to_owned(),
            };
            Ok(Manifest::Project { name, spec })
        }
        (Kind::Profile, _) => {
            let (frontmatter, body) = split_frontmatter(path, text)?;
            let file: ProfileFile = from_yaml(path, frontmatter)?;
            expect_kind(path, &file.kind, Kind::Profile)?;
            Ok(Manifest::Profile(ProfileSpec {
                name: file.name,
                headline: file.headline,
                bio: file.bio,
                email: file.email,
                links: file.links,
                body: body.to_owned(),
            }))
        }
        (Kind::Site, _) => {
            let file: SiteFile = from_yaml(path, text)?;
            expect_kind(path, &file.kind, Kind::Site)?;
            validate_name(path, &file.theme)?;
            Ok(Manifest::Site(SiteSpec {
                title: file.title,
                theme: file.theme,
                banner: file.banner,
                descriptions: file.descriptions,
                blog: file.blog,
            }))
        }
        (Kind::Theme, Some(name)) => {
            let file: ThemeFile = from_yaml(path, text)?;
            expect_kind(path, &file.kind, Kind::Theme)?;
            Ok(Manifest::Theme {
                name,
                spec: ThemeSpec {
                    title: file.title,
                    dark: file.dark,
                    colors: file.colors,
                },
            })
        }
        (kind @ (Kind::Project | Kind::Theme), None) => Err(Error::file(
            path,
            format!("{} without a name", kind.as_str()),
        )),
    }
}

/// Writes a manifest back in its file format.
pub fn render(manifest: &Manifest) -> Result<String, Error> {
    let kind = manifest.kind().as_str().to_owned();
    match manifest {
        Manifest::Project { spec, .. } => {
            let file = ProjectFile {
                kind,
                title: spec.title.clone(),
                tagline: spec.tagline.clone(),
                status: spec.status,
                draft: spec.draft,
                highlight: spec.highlight,
                tags: spec.tags.clone(),
                links: spec.links.clone(),
                github: spec.github.clone(),
                assets: spec.assets.clone(),
            };
            with_frontmatter(&file, &spec.body)
        }
        Manifest::Profile(spec) => {
            let file = ProfileFile {
                kind,
                name: spec.name.clone(),
                headline: spec.headline.clone(),
                bio: spec.bio.clone(),
                email: spec.email.clone(),
                links: spec.links.clone(),
            };
            with_frontmatter(&file, &spec.body)
        }
        Manifest::Site(spec) => to_yaml(&SiteFile {
            kind,
            title: spec.title.clone(),
            theme: spec.theme.clone(),
            banner: spec.banner.clone(),
            descriptions: spec.descriptions.clone(),
            blog: spec.blog.clone(),
        }),
        Manifest::Theme { spec, .. } => to_yaml(&ThemeFile {
            kind,
            title: spec.title.clone(),
            dark: spec.dark,
            colors: spec.colors.clone(),
        }),
    }
}

fn with_frontmatter<T: Serialize>(frontmatter: &T, body: &str) -> Result<String, Error> {
    let yaml = to_yaml(frontmatter)?;
    let newline = if yaml.ends_with('\n') { "" } else { "\n" };
    Ok(format!("---\n{yaml}{newline}---\n{body}"))
}

fn expect_kind(path: &Path, declared: &str, expected: Kind) -> Result<(), Error> {
    if declared == expected.as_str() {
        Ok(())
    } else {
        Err(Error::file(
            path,
            format!(
                "declares kind {declared}, but this path holds a {}",
                expected.as_str()
            ),
        ))
    }
}

fn from_yaml<T: serde::de::DeserializeOwned>(path: &Path, text: &str) -> Result<T, Error> {
    serde_saphyr::from_str(text).map_err(|err| Error::file(path, err.to_string()))
}

fn to_yaml<T: Serialize>(value: &T) -> Result<String, Error> {
    serde_saphyr::to_string(value).map_err(|err| Error::Yaml(err.to_string()))
}

fn split_frontmatter<'a>(path: &Path, source: &'a str) -> Result<(&'a str, &'a str), Error> {
    let rest = source
        .strip_prefix("---\n")
        .or_else(|| source.strip_prefix("---\r\n"))
        .ok_or_else(|| Error::file(path, "missing opening ---"))?;
    rest.split_once("\n---\n")
        .or_else(|| rest.split_once("\r\n---\r\n"))
        .or_else(|| rest.split_once("\n---\r\n"))
        .or_else(|| rest.split_once("\r\n---\n"))
        .ok_or_else(|| Error::file(path, "missing closing ---"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{SITE, THEME};

    const PROJECT: &str = "\
---
kind: Project
title: Atlas
tagline: CubeSat ADCS in Rust
status: active
highlight: 1
tags: [rust, aerospace]
links:
  repo: https://github.com/hvpaiva/atlas
github: hvpaiva/atlas
assets:
  - path: atlas/overview.png
    caption: pipeline
---

Context and **decisions**.
";

    const PROFILE: &str = "\
---
kind: Profile
name: Highlander Paiva
headline: senior platform engineer
bio: writes software.
links:
  github: https://github.com/hvpaiva
---

Design to platform.
";

    fn project(source: &str) -> Result<Manifest, Error> {
        parse(Path::new("projects/atlas.md"), source.as_bytes())
    }

    #[test]
    fn locates_the_layout() {
        let at = |p: &str| locate(Path::new(p)).unwrap();
        assert_eq!(at("site.yaml").unwrap().kind, Kind::Site);
        assert_eq!(at("profile.md").unwrap().kind, Kind::Profile);
        let theme = at("themes/nord.yaml").unwrap();
        assert_eq!(theme.kind, Kind::Theme);
        assert_eq!(theme.name.as_deref(), Some("nord"));
        assert!(at("themes/nord.yml").is_none());
        let project = at("projects/atlas.md").unwrap();
        assert_eq!(project.kind, Kind::Project);
        assert_eq!(project.name.as_deref(), Some("atlas"));
        assert!(at("README.md").is_none());
        assert!(at("projects/notes.txt").is_none());
        assert!(locate(Path::new("projects/Atlas.md")).is_err());
        assert!(locate(Path::new("../site.yaml")).is_err());
        assert!(locate(Path::new("/site.yaml")).is_err());
    }

    #[test]
    fn path_inverts_locate() {
        for (kind, name) in [
            (Kind::Project, Some("atlas")),
            (Kind::Profile, None),
            (Kind::Site, None),
            (Kind::Theme, Some("nord")),
        ] {
            let located = locate(Path::new(&path(kind, name))).unwrap().unwrap();
            assert_eq!(located.kind, kind);
            assert_eq!(located.name.as_deref(), name);
        }
    }

    #[test]
    fn parses_a_project() {
        let Manifest::Project { name, spec } = project(PROJECT).unwrap() else {
            panic!("not a project");
        };
        assert_eq!(name, "atlas");
        assert_eq!(spec.title, "Atlas");
        assert_eq!(spec.highlight, Some(1));
        assert!(!spec.draft);
        assert_eq!(spec.assets[0].caption.as_deref(), Some("pipeline"));
        assert_eq!(spec.body, "\nContext and **decisions**.\n");
    }

    #[test]
    fn project_round_trips() {
        let first = project(PROJECT).unwrap();
        let rendered = render(&first).unwrap();
        let Manifest::Project { spec, .. } = project(&rendered).unwrap() else {
            panic!("not a project: {rendered}");
        };
        let Manifest::Project { spec: before, .. } = first else {
            unreachable!()
        };
        assert_eq!(
            serde_json::to_value(&spec).unwrap(),
            serde_json::to_value(&before).unwrap(),
            "{rendered}"
        );
        assert!(rendered.starts_with("---\nkind: Project\n"), "{rendered}");
    }

    #[test]
    fn profile_round_trips() {
        let path = Path::new("profile.md");
        let first = parse(path, PROFILE.as_bytes()).unwrap();
        let rendered = render(&first).unwrap();
        let Manifest::Profile(spec) = parse(path, rendered.as_bytes()).unwrap() else {
            panic!("not a profile: {rendered}");
        };
        assert_eq!(spec.name, "Highlander Paiva");
        assert_eq!(spec.body, "\nDesign to platform.\n");
    }

    #[test]
    fn site_round_trips() {
        let path = Path::new("site.yaml");
        let first = parse(path, SITE.as_bytes()).unwrap();
        let rendered = render(&first).unwrap();
        let Manifest::Site(spec) = parse(path, rendered.as_bytes()).unwrap() else {
            panic!("not a site: {rendered}");
        };
        assert!(!spec.blog.enabled);
        assert_eq!(spec.theme, "nord");
        let banner = spec.banner.unwrap();
        assert_eq!(banner.art, "##\n #");
        assert_eq!(banner.alt, "EX");
        assert_eq!(spec.descriptions.themes, "Palettes.");
    }

    #[test]
    fn theme_round_trips() {
        let path = Path::new("themes/nord.yaml");
        let first = parse(path, THEME.as_bytes()).unwrap();
        let rendered = render(&first).unwrap();
        let Manifest::Theme { name, spec } = parse(path, rendered.as_bytes()).unwrap() else {
            panic!("not a theme: {rendered}");
        };
        assert_eq!(name, "nord");
        assert_eq!(spec.title, "Nord");
        assert_eq!(spec.colors.bg.as_str(), "#2e3440");
    }

    #[test]
    fn rejects_colors_that_are_not_hex() {
        let path = Path::new("themes/nord.yaml");
        let hostile = THEME.replace("\"#88c0d0\"", "\"red;}</style><script>\"");
        let err = parse(path, hostile.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("#rrggbb"), "{err}");
    }

    #[test]
    fn site_theme_must_be_a_name() {
        let err = parse(
            Path::new("site.yaml"),
            SITE.replace("theme: nord", "theme: ../x").as_bytes(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("name must match"), "{err}");
    }

    #[test]
    fn draft_is_written_only_when_set() {
        let Manifest::Project { name, mut spec } = project(PROJECT).unwrap() else {
            panic!("not a project");
        };
        let rendered = render(&Manifest::Project {
            name: name.clone(),
            spec: spec.clone(),
        })
        .unwrap();
        assert!(!rendered.contains("draft"), "{rendered}");

        spec.draft = true;
        let rendered = render(&Manifest::Project { name, spec }).unwrap();
        let Manifest::Project { spec, .. } = project(&rendered).unwrap() else {
            panic!("not a project");
        };
        assert!(spec.draft);
    }

    #[test]
    fn reads_the_declared_kind() {
        assert_eq!(declared_kind(PROJECT).as_deref(), Some("Project"));
        assert_eq!(declared_kind(THEME).as_deref(), Some("Theme"));
        assert_eq!(declared_kind("title: no kind\n"), None);
        assert_eq!(declared_kind("---\nkind: [\n---\n"), None);
    }

    #[test]
    fn rejects_a_kind_that_disagrees_with_the_path() {
        let err = project(&PROJECT.replace("kind: Project", "kind: Profile")).unwrap_err();
        assert!(err.to_string().contains("declares kind Profile"), "{err}");
    }

    #[test]
    fn rejects_unknown_fields() {
        let err = project(&PROJECT.replace("tagline:", "tagln:")).unwrap_err();
        assert!(err.to_string().contains("projects/atlas.md"), "{err}");

        let err = project(&PROJECT.replace("  repo:", "  rep:")).unwrap_err();
        assert!(err.to_string().contains("projects/atlas.md"), "{err}");
    }

    #[test]
    fn rejects_a_missing_kind() {
        assert!(project(&PROJECT.replace("kind: Project\n", "")).is_err());
    }

    #[test]
    fn rejects_files_outside_the_layout() {
        let err = parse(Path::new("notes.md"), PROFILE.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("not a content file"), "{err}");
    }

    #[test]
    fn requires_frontmatter() {
        assert!(project("no frontmatter").is_err());
        assert!(project("---\nkind: Project\n").is_err());
    }
}
