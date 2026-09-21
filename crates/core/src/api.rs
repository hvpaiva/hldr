//! Wire format of the private API, shared by the server and the CLI.
//!
//! Every resource travels in the same envelope, so the CLI handles any kind
//! the same way. Doc comments on the `spec` and `metadata` fields are what
//! `hldr explain` prints: they are served through [`schemas`].

use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};

use crate::types::{ProfileLinks, ProjectLinks, ProjectStatus};

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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectMetadata {
    /// Identifier, taken from the file name under `content/projects/`.
    pub slug: String,
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
    /// Position among the highlights on the home page. Lower numbers come
    /// first; null leaves the project out of the highlights.
    pub highlight: Option<i64>,
    /// Free-form labels shown next to the project.
    pub tags: Vec<String>,
    pub links: ProjectLinks,
    /// GitHub repository as `owner/name`, the key for repository metrics.
    pub github: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProfileMetadata {
    /// When `profile.yaml` last changed, ISO-8601 UTC.
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
    /// Markdown of the about page.
    pub about: String,
    /// Public contact address.
    pub email: Option<String>,
    pub links: ProfileLinks,
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
    /// Verbs the server supports on this resource.
    pub verbs: Vec<String>,
}

const READ_VERBS: &[&str] = &["get", "describe", "explain"];

/// Resource types the server exposes, in display order.
pub fn resources() -> List<ApiResource> {
    let entry = |name: &str, singular: &str, short_names: &[&str], kind: &str| ApiResource {
        name: name.to_owned(),
        singular: singular.to_owned(),
        short_names: short_names.iter().map(|&s| s.to_owned()).collect(),
        kind: kind.to_owned(),
        verbs: READ_VERBS.iter().map(|&v| v.to_owned()).collect(),
    };
    List {
        kind: "APIResourceList".to_owned(),
        items: vec![
            entry("projects", "project", &["proj", "p"], "Project"),
            entry("profile", "profile", &[], "Profile"),
        ],
    }
}

/// JSON Schema of every resource kind, keyed by kind.
pub fn schemas() -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    out.insert("Project".to_owned(), titled::<Project>("Project"));
    out.insert("Profile".to_owned(), titled::<Profile>("Profile"));
    out
}

fn titled<T: JsonSchema>(title: &str) -> serde_json::Value {
    let mut schema = schema_for!(T);
    schema.insert("title".to_owned(), title.into());
    schema.to_value()
}

// `ProjectSummary` and `Project` share these fields by name; the list and
// the single project must serve the same spec.
macro_rules! project_resource {
    ($item:expr) => {
        Resource {
            kind: "Project".to_owned(),
            metadata: ProjectMetadata {
                slug: $item.slug.clone(),
                created_at: $item.created_at.clone(),
                updated_at: $item.updated_at.clone(),
            },
            spec: ProjectSpec {
                title: $item.title.clone(),
                tagline: $item.tagline.clone(),
                status: $item.status,
                highlight: $item.highlight,
                tags: $item.tags.clone(),
                links: $item.links.clone(),
                github: $item.github_repo.clone(),
            },
            status: empty_status(),
        }
    };
}

impl From<&crate::ProjectSummary> for Project {
    fn from(item: &crate::ProjectSummary) -> Self {
        project_resource!(item)
    }
}

impl From<&crate::Project> for Project {
    fn from(item: &crate::Project) -> Self {
        project_resource!(item)
    }
}

impl From<&crate::Profile> for Profile {
    fn from(item: &crate::Profile) -> Self {
        Resource {
            kind: "Profile".to_owned(),
            metadata: ProfileMetadata {
                updated_at: item.updated_at.clone(),
            },
            spec: ProfileSpec {
                name: item.name.clone(),
                headline: item.headline.clone(),
                bio: item.bio.clone(),
                about: item.about_source.clone(),
                email: item.email.clone(),
                links: item.links.clone(),
            },
            status: empty_status(),
        }
    }
}

/// Projects as a `ProjectList`.
pub fn project_list(items: &[crate::ProjectSummary]) -> List<Project> {
    List {
        kind: "ProjectList".to_owned(),
        items: items.iter().map(Project::from).collect(),
    }
}

fn empty_status() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

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
    fn resource_round_trips() {
        let project = Project {
            kind: "Project".to_owned(),
            metadata: ProjectMetadata {
                slug: "atlas".to_owned(),
                created_at: "2026-01-01T00:00:00Z".to_owned(),
                updated_at: "2026-01-02T00:00:00Z".to_owned(),
            },
            spec: ProjectSpec {
                title: "Atlas".to_owned(),
                tagline: "ADCS".to_owned(),
                status: ProjectStatus::Wip,
                highlight: None,
                tags: vec!["rust".to_owned()],
                links: ProjectLinks::default(),
                github: None,
            },
            status: empty_status(),
        };
        let json = serde_json::to_string(&project).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.spec.status, ProjectStatus::Wip);
        assert_eq!(back.metadata.slug, "atlas");
    }
}
