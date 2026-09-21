//! Wire format of the private API, shared by the server and the CLI.
//!
//! Every resource travels in the same envelope, so the CLI handles any kind
//! the same way. Doc comments on the `spec` and `metadata` fields are what
//! `hldr explain` prints: they are served through [`schemas`].

use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};

use crate::manifest::{Format, Kind};
use crate::types::{AssetSpec, Palette, ProfileLinks, ProjectLinks, ProjectStatus};

/// A resource as the API serves it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Resource<M, S> {
    /// Resource type, such as `Project`.
    pub kind: String,
    /// Identity and bookkeeping, set by the server.
    pub metadata: M,
    /// Desired state: what the content file declares.
    pub spec: S,
    /// Derived state. Never accepted on write.
    pub status: serde_json::Value,
}

/// A collection of resources of one kind.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct List<T> {
    /// List type, such as `ProjectList`.
    pub kind: String,
    pub items: Vec<T>,
}

/// Project as the API serves it.
pub type Project = Resource<ProjectMetadata, ProjectSpec>;

/// Profile as the API serves it.
pub type Profile = Resource<ProfileMetadata, ProfileSpec>;

/// Site configuration as the API serves it.
pub type Site = Resource<SiteMetadata, SiteSpec>;

/// Color scheme as the API serves it.
pub type Theme = Resource<ThemeMetadata, ThemeSpec>;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectMetadata {
    /// Identifier, taken from the file name under `projects/`.
    pub name: String,
    /// When the project was first indexed, ISO-8601 UTC.
    pub created_at: String,
    /// When the project's file last changed, ISO-8601 UTC.
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectSpec {
    /// Display name.
    pub title: String,
    /// One line shown under the title in lists and on the home page.
    pub tagline: String,
    pub status: ProjectStatus,
    /// Kept out of the public site while true.
    pub draft: bool,
    /// Position among the highlights on the home page. Lower numbers come
    /// first; null leaves the project out of the highlights.
    pub highlight: Option<i64>,
    /// Free-form labels shown next to the project.
    pub tags: Vec<String>,
    pub links: ProjectLinks,
    /// GitHub repository as `owner/name`, the key for repository metrics.
    pub github: Option<String>,
    /// Images attached to the project.
    pub assets: Vec<AssetSpec>,
    /// Markdown below the frontmatter.
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProfileMetadata {
    /// When `profile.md` last changed, ISO-8601 UTC.
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProfileSpec {
    /// Full name.
    pub name: String,
    /// Short line under the name.
    pub headline: String,
    /// Paragraph that introduces the profile on the home page.
    pub bio: String,
    /// Public contact address.
    pub email: Option<String>,
    pub links: ProfileLinks,
    /// Markdown below the frontmatter: the about page.
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SiteMetadata {
    /// When a value in `site.yaml` last changed, ISO-8601 UTC.
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SiteSpec {
    /// Site name, shown atop the file tree and after every page title.
    pub title: String,
    /// Theme a visitor sees before picking one; the name of a `Theme`.
    pub theme: String,
    /// ASCII art atop the home page; none when absent.
    pub banner: Option<BannerSpec>,
    pub descriptions: DescriptionsSpec,
    pub blog: BlogSpec,
}

/// ASCII art and what it reads as.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BannerSpec {
    /// The art, kept verbatim.
    pub art: String,
    /// What a screen reader announces instead of the art.
    pub alt: String,
}

/// Lines that introduce the listing pages, also used as their meta description.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescriptionsSpec {
    /// The projects index.
    pub projects: String,
    /// The color scheme picker.
    pub themes: String,
}

/// The blog section of the site.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlogSpec {
    /// Whether the blog is listed and served.
    pub enabled: bool,
}

/// What one pass of the indexer changed.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncReport {
    /// `site.yaml` changed a value.
    pub site_updated: bool,
    /// `profile.md` changed.
    pub profile_updated: bool,
    /// Projects created or rewritten.
    pub projects_upserted: u32,
    /// Projects whose files did not change.
    pub projects_skipped: u32,
    /// Projects whose files are gone.
    pub projects_deleted: u32,
    /// Themes created or rewritten.
    pub themes_upserted: u32,
    /// Themes whose files did not change.
    pub themes_skipped: u32,
    /// Themes whose files are gone.
    pub themes_deleted: u32,
}

/// State of the content sync, as `hldr sync status` shows it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SyncStatus {
    /// Always `SyncStatus`.
    pub kind: String,
    /// Where content comes from, such as `github.com/hvpaiva/hldr-content@main`.
    pub source: String,
    /// GitHub repository as `owner/name`; null for content read from a
    /// directory. Where `hldr` writes.
    #[serde(default)]
    pub repository: Option<String>,
    /// Branch or tag the server follows in `repository`.
    #[serde(default)]
    pub branch: Option<String>,
    /// Content commit being served; null for content read from a directory.
    pub revision: Option<String>,
    /// When the served content was materialized, ISO-8601 UTC.
    pub synced_at: Option<String>,
    /// When a sync last ran, successful or not, ISO-8601 UTC.
    pub last_attempt_at: Option<String>,
    /// Why the last attempt failed; null when it succeeded.
    pub last_error: Option<String>,
    /// What this sync changed. Present only on the response to a sync that
    /// indexed content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<SyncReport>,
}

#[cfg(feature = "store")]
impl SyncStatus {
    pub fn new(
        source: String,
        origin: Option<(String, String)>,
        state: crate::SyncState,
        report: Option<SyncReport>,
    ) -> Self {
        let (repository, branch) = origin.unzip();
        Self {
            kind: "SyncStatus".to_owned(),
            source,
            repository,
            branch,
            revision: state.revision,
            synced_at: state.synced_at,
            last_attempt_at: state.last_attempt_at,
            last_error: state.last_error,
            report,
        }
    }
}

/// Body of `POST /api/v1/sync`. Empty, it syncs whatever the branch points
/// at; with a revision, it waits for the branch to point there first, since
/// GitHub can serve a stale ref for a few seconds after a push.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SyncRequest {
    /// Commit the caller just pushed, as 40 hex digits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ThemeMetadata {
    /// Identifier, taken from the file name under `themes/`; the value the
    /// theme cookie and `/theme/{name}` carry.
    pub name: String,
    /// When the theme's file last changed, ISO-8601 UTC.
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ThemeSpec {
    /// Display name in the picker.
    pub title: String,
    /// Whether the palette is dark; sets the page's `color-scheme`.
    pub dark: bool,
    pub colors: Palette,
}

/// Error body, RFC 9457 (`application/problem+json`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Problem {
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
}

/// A resource type the server exposes, as `hldr api-resources` lists it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApiResource {
    /// Plural name used in paths, such as `projects`.
    pub name: String,
    /// Singular name, used by `hldr explain`.
    pub singular: String,
    pub short_names: Vec<String>,
    /// Key into the document served by [`schemas`].
    pub kind: String,
    /// True when exactly one resource of this kind exists and it has no name.
    pub singleton: bool,
    /// Verbs the CLI supports on this resource.
    pub verbs: Vec<String>,
    /// Where the resource lives in the content repository.
    pub source: Source,
    /// Columns of the table `hldr get` prints.
    pub columns: Vec<Column>,
}

/// The file that declares a resource.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Source {
    /// Path under the content root; `{name}` stands for the resource name.
    pub path: String,
    pub format: Format,
}

/// A table column, in the style of kubectl's printer columns.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Column {
    /// Header text.
    pub name: String,
    /// Field shown, as a kubectl-style JSONPath such as `.spec.status`.
    pub json_path: String,
    /// Shown only with `-o wide`.
    pub wide: bool,
}

const READ_VERBS: &[&str] = &["get", "describe", "explain"];
const WRITE_VERBS: &[&str] = &["edit", "apply", "diff", "patch"];

/// Resource types the server exposes, in display order.
pub fn resources() -> List<ApiResource> {
    List {
        kind: "APIResourceList".to_owned(),
        items: vec![
            entry(
                Kind::Project,
                "projects",
                &["proj", "p"],
                &[
                    ("NAME", ".metadata.name", false),
                    ("STATUS", ".spec.status", false),
                    ("HIGHLIGHT", ".spec.highlight", false),
                    ("DRAFT", ".spec.draft", false),
                    ("TAGLINE", ".spec.tagline", true),
                    ("TAGS", ".spec.tags", true),
                    ("UPDATED", ".metadata.updated_at", true),
                ],
            ),
            entry(
                Kind::Profile,
                "profile",
                &[],
                &[
                    ("NAME", ".spec.name", false),
                    ("HEADLINE", ".spec.headline", false),
                    ("EMAIL", ".spec.email", true),
                    ("UPDATED", ".metadata.updated_at", true),
                ],
            ),
            entry(
                Kind::Site,
                "site",
                &[],
                &[
                    ("TITLE", ".spec.title", false),
                    ("THEME", ".spec.theme", false),
                    ("BLOG", ".spec.blog.enabled", false),
                    ("UPDATED", ".metadata.updated_at", true),
                ],
            ),
            entry(
                Kind::Theme,
                "themes",
                &["th"],
                &[
                    ("NAME", ".metadata.name", false),
                    ("TITLE", ".spec.title", false),
                    ("DARK", ".spec.dark", false),
                    ("BG", ".spec.colors.bg", true),
                    ("ACCENT", ".spec.colors.accent", true),
                    ("UPDATED", ".metadata.updated_at", true),
                ],
            ),
        ],
    }
}

fn entry(
    kind: Kind,
    name: &str,
    short_names: &[&str],
    columns: &[(&str, &str, bool)],
) -> ApiResource {
    let owned = |items: &[&str]| items.iter().map(|&s| s.to_owned()).collect::<Vec<_>>();
    let mut verbs = owned(READ_VERBS);
    verbs.extend(owned(WRITE_VERBS));
    if !kind.is_singleton() {
        verbs.push("delete".to_owned());
    }
    ApiResource {
        name: name.to_owned(),
        singular: kind.as_str().to_lowercase(),
        short_names: owned(short_names),
        kind: kind.as_str().to_owned(),
        singleton: kind.is_singleton(),
        verbs,
        source: Source {
            path: kind.path_template().to_owned(),
            format: kind.format(),
        },
        columns: columns
            .iter()
            .map(|&(name, json_path, wide)| Column {
                name: name.to_owned(),
                json_path: json_path.to_owned(),
                wide,
            })
            .collect(),
    }
}

/// JSON Schema of every resource kind, keyed by kind.
pub fn schemas() -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    out.insert("Project".to_owned(), titled::<Project>("Project"));
    out.insert("Profile".to_owned(), titled::<Profile>("Profile"));
    out.insert("Site".to_owned(), titled::<Site>("Site"));
    out.insert("Theme".to_owned(), titled::<Theme>("Theme"));
    out
}

fn titled<T: JsonSchema>(title: &str) -> serde_json::Value {
    let mut schema = schema_for!(T);
    schema.insert("title".to_owned(), title.into());
    schema.to_value()
}

#[cfg(feature = "store")]
mod from_store {
    use super::*;

    fn empty_status() -> serde_json::Value {
        serde_json::Value::Object(serde_json::Map::new())
    }

    impl From<&crate::Project> for Project {
        fn from(item: &crate::Project) -> Self {
            Resource {
                kind: Kind::Project.as_str().to_owned(),
                metadata: ProjectMetadata {
                    name: item.slug.clone(),
                    created_at: item.created_at.clone(),
                    updated_at: item.updated_at.clone(),
                },
                spec: ProjectSpec {
                    title: item.title.clone(),
                    tagline: item.tagline.clone(),
                    status: item.status,
                    draft: item.draft,
                    highlight: item.highlight,
                    tags: item.tags.clone(),
                    links: item.links.clone(),
                    github: item.github_repo.clone(),
                    assets: item.assets.clone(),
                    body: item.body_source.clone(),
                },
                status: empty_status(),
            }
        }
    }

    impl From<&crate::Profile> for Profile {
        fn from(item: &crate::Profile) -> Self {
            Resource {
                kind: Kind::Profile.as_str().to_owned(),
                metadata: ProfileMetadata {
                    updated_at: item.updated_at.clone(),
                },
                spec: ProfileSpec {
                    name: item.name.clone(),
                    headline: item.headline.clone(),
                    bio: item.bio.clone(),
                    email: item.email.clone(),
                    links: item.links.clone(),
                    body: item.about_source.clone(),
                },
                status: empty_status(),
            }
        }
    }

    impl From<&crate::SiteConfig> for Site {
        fn from(item: &crate::SiteConfig) -> Self {
            Resource {
                kind: Kind::Site.as_str().to_owned(),
                metadata: SiteMetadata {
                    updated_at: item.updated_at.clone(),
                },
                spec: SiteSpec {
                    title: item.title.clone(),
                    theme: item.theme.clone(),
                    banner: item.banner.clone(),
                    descriptions: item.descriptions.clone(),
                    blog: BlogSpec {
                        enabled: item.blog_enabled,
                    },
                },
                status: empty_status(),
            }
        }
    }

    impl From<&crate::Theme> for Theme {
        fn from(item: &crate::Theme) -> Self {
            Resource {
                kind: Kind::Theme.as_str().to_owned(),
                metadata: ThemeMetadata {
                    name: item.slug.clone(),
                    updated_at: item.updated_at.clone(),
                },
                spec: ThemeSpec {
                    title: item.title.clone(),
                    dark: item.dark,
                    colors: item.colors.clone(),
                },
                status: empty_status(),
            }
        }
    }

    /// Themes as a `ThemeList`.
    pub fn theme_list(items: &[crate::Theme]) -> List<Theme> {
        List {
            kind: "ThemeList".to_owned(),
            items: items.iter().map(Theme::from).collect(),
        }
    }

    /// Projects as a `ProjectList`.
    pub fn project_list(items: &[crate::Project]) -> List<Project> {
        List {
            kind: "ProjectList".to_owned(),
            items: items.iter().map(Project::from).collect(),
        }
    }
}

#[cfg(feature = "store")]
pub use from_store::{project_list, theme_list};

#[cfg(test)]
mod tests {
    use super::*;

    fn field<'a>(schema: &'a serde_json::Value, path: &[&str]) -> &'a serde_json::Value {
        let defs = &schema["$defs"];
        let mut node = schema;
        for name in path {
            node = &node["properties"][*name];
            if let Some(reference) = node["$ref"].as_str() {
                let def = reference.trim_start_matches("#/$defs/");
                node = &defs[def];
            }
        }
        node
    }

    #[test]
    fn field_docs_reach_the_schema() {
        let schemas = schemas();
        let project = &schemas["Project"];
        assert_eq!(project["title"], "Project");

        let highlight = field(project, &["spec", "highlight"]);
        let doc = highlight["description"].as_str().unwrap();
        assert!(doc.starts_with("Position among the highlights"), "{doc}");

        let repo = field(project, &["spec", "links", "repo"]);
        assert_eq!(repo["description"], "Source repository URL.");

        let status = field(project, &["spec", "status"]);
        assert_eq!(status["description"], "Where a project stands.");

        let blog = field(&schemas["Site"], &["spec", "blog", "enabled"]);
        assert_eq!(
            blog["description"],
            "Whether the blog is listed and served."
        );
    }

    #[test]
    fn every_resource_has_a_schema() {
        let schemas = schemas();
        let catalog = resources();
        assert_eq!(catalog.items.len(), schemas.len());
        for resource in &catalog.items {
            assert!(schemas.contains_key(&resource.kind), "{}", resource.kind);
        }
    }

    #[test]
    fn every_column_names_a_schema_field() {
        let schemas = schemas();
        for resource in resources().items {
            let schema = &schemas[&resource.kind];
            for column in &resource.columns {
                let path: Vec<&str> = column
                    .json_path
                    .trim_start_matches('.')
                    .split('.')
                    .collect();
                assert!(
                    field(schema, &path).is_object(),
                    "{}: {}",
                    resource.kind,
                    column.json_path
                );
            }
        }
    }

    #[test]
    fn only_named_kinds_can_be_deleted() {
        for resource in resources().items {
            let deletable = resource.verbs.iter().any(|verb| verb == "delete");
            assert_eq!(deletable, !resource.singleton, "{}", resource.kind);
        }
    }

    #[test]
    fn resource_round_trips() {
        let project = Project {
            kind: "Project".to_owned(),
            metadata: ProjectMetadata {
                name: "atlas".to_owned(),
                created_at: "2026-01-01T00:00:00Z".to_owned(),
                updated_at: "2026-01-02T00:00:00Z".to_owned(),
            },
            spec: ProjectSpec {
                title: "Atlas".to_owned(),
                tagline: "ADCS".to_owned(),
                status: ProjectStatus::Wip,
                draft: true,
                highlight: None,
                tags: vec!["rust".to_owned()],
                links: ProjectLinks::default(),
                github: None,
                assets: Vec::new(),
                body: "Body.\n".to_owned(),
            },
            status: serde_json::json!({}),
        };
        let json = serde_json::to_string(&project).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.spec.status, ProjectStatus::Wip);
        assert!(back.spec.draft);
        assert_eq!(back.metadata.name, "atlas");
    }
}
