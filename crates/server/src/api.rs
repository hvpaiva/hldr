use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use hldr_core::{Profile, Project, ProjectSummary};
use serde::Serialize;

#[derive(Serialize)]
struct Resource<M, S> {
    kind: &'static str,
    metadata: M,
    spec: S,
    status: serde_json::Value,
}

#[derive(Serialize)]
struct List<T> {
    kind: &'static str,
    items: Vec<T>,
}

#[derive(Serialize)]
struct ProjectMeta<'a> {
    slug: &'a str,
    created_at: &'a str,
    updated_at: &'a str,
}

#[derive(Serialize)]
struct ProjectSpec<'a> {
    title: &'a str,
    tagline: &'a str,
    status: &'a str,
    highlight: Option<i64>,
    tags: &'a [String],
    links: &'a hldr_core::types::ProjectLinks,
    github: Option<&'a str>,
}

#[derive(Serialize)]
struct ProfileMeta<'a> {
    updated_at: &'a str,
}

#[derive(Serialize)]
struct ProfileSpec<'a> {
    name: &'a str,
    headline: &'a str,
    bio: &'a str,
    about: &'a str,
    email: Option<&'a str>,
    links: &'a hldr_core::types::ProfileLinks,
}

#[derive(Serialize)]
struct Problem {
    #[serde(rename = "type")]
    kind: &'static str,
    title: &'static str,
    status: u16,
    detail: String,
}

pub fn projects(items: &[ProjectSummary]) -> impl IntoResponse {
    Json(List {
        kind: "ProjectList",
        items: items.iter().map(project_summary).collect(),
    })
}

pub fn project(item: &Project) -> impl IntoResponse {
    Json(Resource {
        kind: "Project",
        metadata: ProjectMeta {
            slug: &item.slug,
            created_at: &item.created_at,
            updated_at: &item.updated_at,
        },
        spec: ProjectSpec {
            title: &item.title,
            tagline: &item.tagline,
            status: item.status.as_str(),
            highlight: item.highlight,
            tags: &item.tags,
            links: &item.links,
            github: item.github_repo.as_deref(),
        },
        status: serde_json::json!({}),
    })
}

pub fn profile(item: &Profile) -> impl IntoResponse {
    Json(Resource {
        kind: "Profile",
        metadata: ProfileMeta {
            updated_at: &item.updated_at,
        },
        spec: ProfileSpec {
            name: &item.name,
            headline: &item.headline,
            bio: &item.bio,
            about: &item.about_source,
            email: item.email.as_deref(),
            links: &item.links,
        },
        status: serde_json::json!({}),
    })
}

pub fn not_found(detail: impl Into<String>) -> Response {
    problem(StatusCode::NOT_FOUND, "Not Found", detail)
}

pub fn problem(status: StatusCode, title: &'static str, detail: impl Into<String>) -> Response {
    let body = Problem {
        kind: "about:blank",
        title,
        status: status.as_u16(),
        detail: detail.into(),
    };
    (
        status,
        [("content-type", "application/problem+json")],
        Json(body),
    )
        .into_response()
}

fn project_summary(item: &ProjectSummary) -> Resource<ProjectMeta<'_>, ProjectSpec<'_>> {
    Resource {
        kind: "Project",
        metadata: ProjectMeta {
            slug: &item.slug,
            created_at: &item.created_at,
            updated_at: &item.updated_at,
        },
        spec: ProjectSpec {
            title: &item.title,
            tagline: &item.tagline,
            status: item.status.as_str(),
            highlight: item.highlight,
            tags: &item.tags,
            links: &item.links,
            github: item.github_repo.as_deref(),
        },
        status: serde_json::json!({}),
    }
}
