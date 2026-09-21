use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use hldr_core::api::{self, Problem};
use tower_http::trace::TraceLayer;

use crate::{AppError, AppState, healthz};

/// The private API. Reachable only through the tailnet (#30); the public
/// listener never routes here.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/api-resources", get(api_resources))
        .route("/api/v1/schema", get(schema))
        .route("/api/v1/projects", get(projects))
        .route("/api/v1/projects/{slug}", get(project))
        .route("/api/v1/profile", get(profile))
        .route("/api/v1/posts", get(posts))
        .route("/api/v1/posts/{slug}", get(post))
        .fallback(fallback)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn api_resources() -> Json<api::List<api::ApiResource>> {
    Json(api::resources())
}

async fn schema() -> Json<serde_json::Map<String, serde_json::Value>> {
    Json(api::schemas())
}

async fn projects(State(state): State<AppState>) -> Result<Response, AppError> {
    let items = state.db.projects().await?;
    Ok(Json(api::project_list(&items)).into_response())
}

async fn project(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, AppError> {
    let Some(project) = state.db.project(&slug).await? else {
        return Ok(not_found(format!("project '{slug}' not found")));
    };
    Ok(Json(api::Project::from(&project)).into_response())
}

async fn profile(State(state): State<AppState>) -> Result<Response, AppError> {
    let profile = state.db.profile().await?;
    Ok(Json(api::Profile::from(&profile)).into_response())
}

async fn posts(State(state): State<AppState>) -> Result<Response, AppError> {
    if state.db.blog_enabled().await? {
        return Ok(not_found("no posts"));
    }
    Ok(not_found("blog is disabled"))
}

async fn post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, AppError> {
    if state.db.blog_enabled().await? {
        return Ok(not_found(format!("post '{slug}' not found")));
    }
    Ok(not_found("blog is disabled"))
}

async fn fallback() -> Response {
    not_found("no such endpoint")
}

pub fn not_found(detail: impl Into<String>) -> Response {
    problem(StatusCode::NOT_FOUND, "Not Found", detail)
}

pub fn problem(status: StatusCode, title: &str, detail: impl Into<String>) -> Response {
    let body = Problem {
        kind: "about:blank".to_owned(),
        title: title.to_owned(),
        status: status.as_u16(),
        detail: detail.into(),
    };
    (
        status,
        [(header::CONTENT_TYPE, "application/problem+json")],
        Json(body),
    )
        .into_response()
}
