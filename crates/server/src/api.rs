use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use hldr_core::api::{self, Problem};
use tower_http::trace::TraceLayer;

use crate::content::SyncError;
use crate::{AppState, healthz};

/// A handler failure, answered as a problem like every other API error.
pub struct ApiError(hldr_core::Error);

impl From<hldr_core::Error> for ApiError {
    fn from(value: hldr_core::Error) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        tracing::error!(error = %self.0, "handler failed");
        problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
            "internal error",
        )
    }
}

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
        .route("/api/v1/site", get(site))
        .route("/api/v1/themes", get(themes))
        .route("/api/v1/themes/{name}", get(theme))
        .route("/api/v1/sync", get(sync_status).post(sync))
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

async fn projects(State(state): State<AppState>) -> Result<Response, ApiError> {
    let items = state.db.all_projects().await?;
    Ok(Json(api::project_list(&items)).into_response())
}

async fn project(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, ApiError> {
    let Some(project) = state.db.any_project(&slug).await? else {
        return Ok(not_found(format!("project '{slug}' not found")));
    };
    Ok(Json(api::Project::from(&project)).into_response())
}

async fn profile(State(state): State<AppState>) -> Result<Response, ApiError> {
    let profile = state.db.profile().await?;
    Ok(Json(api::Profile::from(&profile)).into_response())
}

async fn site(State(state): State<AppState>) -> Result<Response, ApiError> {
    let site = state.db.site_config().await?;
    Ok(Json(api::Site::from(&site)).into_response())
}

async fn themes(State(state): State<AppState>) -> Result<Response, ApiError> {
    let items = state.db.themes().await?;
    Ok(Json(api::theme_list(&items)).into_response())
}

async fn theme(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    let Some(theme) = state.db.theme(&name).await? else {
        return Ok(not_found(format!("theme '{name}' not found")));
    };
    Ok(Json(api::Theme::from(&theme)).into_response())
}

async fn sync_status(State(state): State<AppState>) -> Result<Response, ApiError> {
    Ok(Json(status(&state, None).await?).into_response())
}

/// Brings the served content to the source's current revision and answers
/// once it is, or with the reason it could not be.
async fn sync(State(state): State<AppState>) -> Result<Response, ApiError> {
    match state.syncer.sync(false).await {
        Ok(report) => Ok(Json(status(&state, report).await?).into_response()),
        Err(SyncError::Fetch(message)) => Ok(problem(
            StatusCode::BAD_GATEWAY,
            "Bad Gateway",
            format!("content fetch failed: {message}"),
        )),
        Err(SyncError::Index(error @ hldr_core::Error::Sqlx(_)))
        | Err(SyncError::Index(error @ hldr_core::Error::Migrate(_)))
        | Err(SyncError::Index(error @ hldr_core::Error::Io(_))) => Err(error.into()),
        Err(SyncError::Index(error)) => Ok(problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Unprocessable Content",
            format!("content is invalid, previous revision still served: {error}"),
        )),
    }
}

async fn status(
    state: &AppState,
    report: Option<hldr_core::SyncReport>,
) -> Result<api::SyncStatus, ApiError> {
    let sync = state.db.sync_state().await?;
    Ok(api::SyncStatus::new(
        state.syncer.source().describe(),
        sync,
        report,
    ))
}

async fn posts(State(state): State<AppState>) -> Result<Response, ApiError> {
    if state.db.blog_enabled().await? {
        return Ok(not_found("no posts"));
    }
    Ok(not_found("blog is disabled"))
}

async fn post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, ApiError> {
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
