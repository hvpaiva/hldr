use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use hldr_core::api::{self, PageType, Problem};
use hldr_core::store;
use serde::Deserialize;
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
        .route("/api/v1/sync", get(sync_status).post(sync))
        .route("/api/v1/events", get(events))
        .route("/api/v1/events/{name}", get(event))
        .route("/api/v1/server", get(server))
        .route("/api/v1/metrics/{view}", get(metrics))
        .route("/api/v1/{resource}", get(list))
        .route("/api/v1/{resource}/{name}", get(item))
        .fallback(fallback)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// The page types, each with the collection that is its home.
async fn page_types(
    state: &AppState,
) -> Result<(Vec<store::PageKind>, Vec<store::Collection>), ApiError> {
    Ok((state.db.page_kinds().await?, state.db.collections().await?))
}

fn paired<'a>(
    kinds: &'a [store::PageKind],
    collections: &'a [store::Collection],
) -> Vec<PageType<'a>> {
    kinds
        .iter()
        .filter_map(|kind| {
            let home = collections
                .iter()
                .find(|collection| collection.spec.kind == kind.spec.names.kind)?;
            Some(PageType {
                collection: &home.name,
                spec: &kind.spec,
            })
        })
        .collect()
}

async fn api_resources(State(state): State<AppState>) -> Result<Response, ApiError> {
    let (kinds, collections) = page_types(&state).await?;
    Ok(Json(api::resources(&paired(&kinds, &collections))).into_response())
}

async fn schema(State(state): State<AppState>) -> Result<Response, ApiError> {
    let (kinds, collections) = page_types(&state).await?;
    Ok(Json(api::schemas(&paired(&kinds, &collections))).into_response())
}

/// `/api/v1/<resource>`: a singleton, or every resource of a kind. Built-in
/// resources come first, then the page types content declares.
async fn list(
    State(state): State<AppState>,
    Path(resource): Path<String>,
) -> Result<Response, ApiError> {
    let db = &state.db;
    let json = match resource.as_str() {
        "site" => Json(api::Site::from(&db.site_config().await?)).into_response(),
        "profile" => Json(api::Profile::from(&db.profile().await?)).into_response(),
        "nav" => match db.nav().await? {
            Some(nav) => Json(api::Nav::from(&nav)).into_response(),
            None => not_found("the nav is not synced yet"),
        },
        "pagekinds" => {
            let list: api::List<api::PageKind> = api::list("PageKind", &db.page_kinds().await?);
            Json(list).into_response()
        }
        "collections" => {
            let list: api::List<api::Collection> =
                api::list("Collection", &db.collections().await?);
            Json(list).into_response()
        }
        "themes" => {
            let list: api::List<api::Theme> = api::list("Theme", &db.themes().await?);
            Json(list).into_response()
        }
        "pages" => {
            let pages = db.pages().await?;
            let list: api::List<api::Page> =
                api::list("Page", pages.iter().filter(|page| page.page.kind == "Page"));
            Json(list).into_response()
        }
        plural => {
            let (kinds, collections) = page_types(&state).await?;
            let Some(page_type) = paired(&kinds, &collections)
                .into_iter()
                .find(|page_type| page_type.spec.names.plural == plural)
            else {
                return Ok(not_found(format!("no resource type {plural}")));
            };
            let order = collections
                .iter()
                .find(|collection| collection.name == page_type.collection)
                .and_then(|collection| collection.spec.order.clone());
            let pages = db.pages().await?;
            let mut members: Vec<&store::Page> = pages
                .iter()
                .filter(|page| page.page.kind == page_type.spec.names.kind)
                .collect();
            store::order(&mut members, order.as_deref());
            let list: api::List<api::Page> = api::list(&page_type.spec.names.kind, members);
            Json(list).into_response()
        }
    };
    Ok(json)
}

/// `/api/v1/<resource>/<name>`: one named resource, drafts included.
async fn item(
    State(state): State<AppState>,
    Path((resource, name)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let db = &state.db;
    let missing = |singular: &str| not_found(format!("{singular} '{name}' not found"));
    let json = match resource.as_str() {
        "site" | "profile" | "nav" | "server" => {
            return Ok(not_found(format!(
                "{resource} is a singleton: it takes no name"
            )));
        }
        "pagekinds" => match db.page_kinds().await?.iter().find(|k| k.name == name) {
            Some(kind) => Json(api::PageKind::from(kind)).into_response(),
            None => missing("pagekind"),
        },
        "collections" => match db.collections().await?.iter().find(|c| c.name == name) {
            Some(collection) => Json(api::Collection::from(collection)).into_response(),
            None => missing("collection"),
        },
        "themes" => match db.theme(&name).await? {
            Some(theme) => Json(api::Theme::from(&theme)).into_response(),
            None => missing("theme"),
        },
        "pages" => match db.page("Page", &name).await? {
            Some(page) => Json(api::Page::from(&page)).into_response(),
            None => missing("page"),
        },
        plural => {
            let (kinds, collections) = page_types(&state).await?;
            let Some(page_type) = paired(&kinds, &collections)
                .into_iter()
                .find(|page_type| page_type.spec.names.plural == plural)
            else {
                return Ok(not_found(format!("no resource type {plural}")));
            };
            match db.page(&page_type.spec.names.kind, &name).await? {
                Some(page) => Json(api::Page::from(&page)).into_response(),
                None => missing(&page_type.spec.names.singular),
            }
        }
    };
    Ok(json)
}

#[derive(Deserialize)]
struct EventsQuery {
    after: Option<i64>,
}

/// `/api/v1/events[?after=SEQ]`: every event, or those written after a
/// stream position, which is how `hldr get events --watch` polls.
async fn events(
    State(state): State<AppState>,
    query: Result<Query<EventsQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Ok(Query(query)) = query else {
        return Ok(problem(
            StatusCode::BAD_REQUEST,
            "Bad Request",
            "after must be a stream position, a whole number",
        ));
    };
    Ok(Json(state.db.events(query.after).await?).into_response())
}

async fn event(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    let found = match name.parse() {
        Ok(id) => state.db.event(id).await?,
        Err(_) => None,
    };
    Ok(match found {
        Some(event) => Json(event).into_response(),
        None => not_found(format!("event '{name}' not found")),
    })
}

#[derive(Deserialize)]
struct MetricsQuery {
    since: Option<String>,
}

/// `/api/v1/metrics/{daily,pages,referrers}[?since=7d|30d|all]`: what the
/// site served, counted without tracking. The last minute may not have
/// been flushed yet.
async fn metrics(
    State(state): State<AppState>,
    Path(view): Path<String>,
    query: Result<Query<MetricsQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let since = match query.map(|Query(query)| query.since) {
        Ok(None) => api::Since::default(),
        Ok(Some(since)) => match since.parse() {
            Ok(since) => since,
            Err(message) => return Ok(problem(StatusCode::BAD_REQUEST, "Bad Request", message)),
        },
        Err(rejection) => {
            return Ok(problem(
                StatusCode::BAD_REQUEST,
                "Bad Request",
                rejection.body_text(),
            ));
        }
    };
    let db = &state.db;
    Ok(match view.as_str() {
        "daily" => Json(db.daily_metrics(since).await?).into_response(),
        "pages" => Json(db.page_metrics(since).await?).into_response(),
        "referrers" => Json(db.referrer_metrics(since).await?).into_response(),
        other => not_found(format!(
            "no metrics named '{other}': daily, pages or referrers"
        )),
    })
}

/// `/api/v1/server`: the running process, its content and its database.
async fn server(State(state): State<AppState>) -> Result<Response, ApiError> {
    let sync = state.db.sync_state().await?;
    let size = state.db.size()?;
    let server = api::Server {
        kind: "Server".to_owned(),
        metadata: api::SingletonMetadata::default(),
        status: api::ServerStatus {
            version: hldr_core::VERSION.to_owned(),
            revision: hldr_core::REVISION.to_owned(),
            started_at: state.started.at.clone(),
            uptime: uptime(state.started.instant.elapsed()),
            content: api::ContentState {
                source: state.syncer.source().describe(),
                revision: sync.revision,
                synced_at: sync.synced_at,
                last_attempt_at: sync.last_attempt_at,
                last_error: sync.last_error,
            },
            database: api::DatabaseState {
                bytes: size.bytes,
                wal_bytes: size.wal_bytes,
            },
            github: state.syncer.quota(),
        },
    };
    Ok(Json(server).into_response())
}

/// Such as `3d4h`, `2h15m` or `40s`: the two largest units, as kubectl
/// writes ages.
fn uptime(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let (days, hours, minutes, seconds) = (
        seconds / 86_400,
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
    );
    match (days, hours, minutes) {
        (0, 0, 0) => format!("{seconds}s"),
        (0, 0, _) => format!("{minutes}m{seconds}s"),
        (0, _, _) => format!("{hours}h{minutes}m"),
        _ => format!("{days}d{hours}h"),
    }
}

#[cfg(test)]
#[test]
fn uptime_reads_as_an_age() {
    let cases = [
        (0, "0s"),
        (59, "59s"),
        (61, "1m1s"),
        (3600, "1h0m"),
        (7_380, "2h3m"),
        (86_400, "1d0h"),
        (4 * 86_400 + 3 * 3600 + 59, "4d3h"),
    ];
    for (seconds, expected) in cases {
        assert_eq!(uptime(Duration::from_secs(seconds)), expected, "{seconds}");
    }
}

async fn sync_status(State(state): State<AppState>) -> Result<Response, ApiError> {
    Ok(Json(status(&state, None).await?).into_response())
}

/// Brings the served content to the source's current revision and answers
/// once it is, or with the reason it could not be.
async fn sync(
    State(state): State<AppState>,
    request: Option<Json<api::SyncRequest>>,
) -> Result<Response, ApiError> {
    let expected = request.and_then(|Json(request)| request.revision);
    match state.syncer.sync_to(false, expected.as_deref()).await {
        Ok(report) => Ok(Json(status(&state, report).await?).into_response()),
        Err(SyncError::Stale(message)) => Ok(problem(StatusCode::CONFLICT, "Conflict", message)),
        Err(SyncError::Fetch(message) | SyncError::RateLimited(message)) => Ok(problem(
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
        state.syncer.source().origin(),
        sync,
        report,
    ))
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
