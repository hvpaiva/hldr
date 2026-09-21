use std::net::SocketAddr;
use std::path::PathBuf;

use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use hldr_core::{Db, Health};
use tokio::signal::unix::{SignalKind, signal};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

mod api;
mod html;
mod listen;
mod source;
mod theme;

const DEFAULT_ADDR: &str = "127.0.0.1:8080";
const DEFAULT_API_ADDR: &str = "127.0.0.1:8081";
const STYLE: &str = include_str!("../assets/style.css");
const KEYS_JS: &str = include_str!("../assets/keys.js");
const VIM_JS: &str = include_str!("../assets/vim.js");
const FAVICON_ICO: &[u8] = include_bytes!("../assets/favicon.ico");

#[derive(Clone)]
struct AppState {
    db: Db,
    site: Site,
}

#[derive(Clone)]
pub struct Site {
    pub origin: String,
    pub host: String,
}

struct AppError(hldr_core::Error);

impl From<hldr_core::Error> for AppError {
    fn from(value: hldr_core::Error) -> Self {
        Self(value)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!(error = %self.0, "handler failed");
        (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_env("HLDR_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let addr: SocketAddr = std::env::var("HLDR_ADDR")
        .unwrap_or_else(|_| DEFAULT_ADDR.to_owned())
        .parse()
        .expect("HLDR_ADDR must be a socket address");
    let api_addr: listen::ListenAddr = std::env::var("HLDR_API_ADDR")
        .unwrap_or_else(|_| DEFAULT_API_ADDR.to_owned())
        .parse()
        .expect("HLDR_API_ADDR must be host:port or unix:/path");

    let db_path = database_path();
    let content_dir = content_dir();

    let db = Db::open(&db_path).await.expect("failed to open database");
    let report = hldr_core::index::sync(db.pool(), &content_dir)
        .await
        .expect("failed to index content");
    tracing::info!(
        profile_updated = report.profile_updated,
        projects_upserted = report.projects_upserted,
        projects_skipped = report.projects_skipped,
        projects_deleted = report.projects_deleted,
        db = %db_path.display(),
        content = %content_dir.display(),
        "content indexed"
    );

    let state = AppState {
        db,
        site: Site::from_env(),
    };
    let site = site_router(state.clone());
    let private = api::router(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind HLDR_ADDR");

    let (stop, stopped) = tokio::sync::watch::channel(());
    tokio::spawn(async move {
        terminate().await;
        let _ = stop.send(());
    });
    let shutdown = move || {
        let mut stopped = stopped.clone();
        async move {
            let _ = stopped.changed().await;
        }
    };

    // Bound last, once the database and the index are ready: on a unix
    // socket this takes the path over from the container being replaced.
    let api_server = match &api_addr {
        listen::ListenAddr::Tcp(api_tcp) => {
            let api_listener = tokio::net::TcpListener::bind(api_tcp)
                .await
                .expect("failed to bind HLDR_API_ADDR");
            tokio::spawn(
                axum::serve(api_listener, private)
                    .with_graceful_shutdown(shutdown())
                    .into_future(),
            )
        }
        listen::ListenAddr::Unix(path) => {
            let api_listener = listen::bind_unix(path).expect("failed to bind HLDR_API_ADDR");
            tokio::spawn(
                axum::serve(api_listener, private)
                    .with_graceful_shutdown(shutdown())
                    .into_future(),
            )
        }
    };

    tracing::info!(
        %addr,
        %api_addr,
        version = hldr_core::VERSION,
        revision = hldr_core::REVISION,
        "hldr-server listening"
    );

    axum::serve(listener, site)
        .with_graceful_shutdown(shutdown())
        .await
        .expect("server error");
    api_server
        .await
        .expect("api server task panicked")
        .expect("api server error");
}

fn site_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/projects", get(projects))
        .route("/projects/{slug}", get(project))
        .route("/about", get(about))
        .route("/profile", get(profile_page))
        .route("/help", get(help_page))
        .route("/blog", get(blog))
        .route("/blog/{slug}", get(blog_post))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/theme", get(themes))
        .route("/theme/{slug}", get(set_theme))
        .route("/style.css", get(style_sheet))
        .route("/keys.js", get(keys_js))
        .route("/vim.js", get(vim_js))
        .route("/favicon.svg", get(favicon))
        .route("/favicon.ico", get(favicon_ico))
        .route("/sitemap.xml", get(sitemap))
        .route("/robots.txt", get(robots))
        .fallback(not_found)
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::STRICT_TRANSPORT_SECURITY,
        HeaderValue::from_static("max-age=63072000; includeSubDomains"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; \
             base-uri 'self'; frame-ancestors 'none'; form-action 'self'; upgrade-insecure-requests",
        ),
    );
    response
}

impl Site {
    fn from_env() -> Self {
        let origin = std::env::var("HLDR_ORIGIN")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".to_owned())
            .trim_end_matches('/')
            .to_owned();
        let host = origin
            .split_once("://")
            .map(|(_, rest)| rest.to_owned())
            .unwrap_or_else(|| origin.clone());
        Self { origin, host }
    }
}

fn database_path() -> PathBuf {
    std::env::var("HLDR_DATABASE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("hldr.db"))
}

fn content_dir() -> PathBuf {
    std::env::var("HLDR_CONTENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("content"))
}

async fn healthz() -> Json<Health> {
    Json(Health::ok())
}

async fn readyz(State(state): State<AppState>) -> Response {
    match state.db.ping().await {
        Ok(()) => Json(Health::ok()).into_response(),
        Err(error) => {
            tracing::error!(%error, "readyz failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "unready",
                    "version": hldr_core::VERSION,
                    "revision": hldr_core::REVISION,
                })),
            )
                .into_response()
        }
    }
}

async fn home(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let profile = state.db.profile().await?;
    let highlighted = state.db.highlighted_projects().await?;
    let (projects, blog_enabled) = tree_data(&state).await?;
    let tree = html::Tree {
        projects: &projects,
        blog_enabled,
    };
    Ok(html::home(
        &state.site,
        &tree,
        &profile,
        &highlighted,
        theme::Theme::from_headers(&headers),
    )
    .into_response())
}

async fn projects(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let (projects, blog_enabled) = tree_data(&state).await?;
    let tree = html::Tree {
        projects: &projects,
        blog_enabled,
    };
    Ok(html::projects_index(
        &state.site,
        &tree,
        "Work worth opening. Context lives on the project page, not the README.",
        theme::Theme::from_headers(&headers),
    )
    .into_response())
}

async fn about(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let profile = state.db.profile().await?;
    let (projects, blog_enabled) = tree_data(&state).await?;
    let tree = html::Tree {
        projects: &projects,
        blog_enabled,
    };
    Ok(html::about(
        &state.site,
        &tree,
        &profile,
        theme::Theme::from_headers(&headers),
    )
    .into_response())
}

async fn themes(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let (projects, blog_enabled) = tree_data(&state).await?;
    let tree = html::Tree {
        projects: &projects,
        blog_enabled,
    };
    Ok(
        html::themes_index(&state.site, &tree, theme::Theme::from_headers(&headers))
            .into_response(),
    )
}

async fn project(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(project) = state.db.project(&slug).await? else {
        return Ok(render_not_found(
            &state,
            &headers,
            &format!("/projects/{slug}"),
            &format!("{slug}: no such project"),
        )
        .await);
    };
    let (projects, blog_enabled) = tree_data(&state).await?;
    let tree = html::Tree {
        projects: &projects,
        blog_enabled,
    };
    Ok(html::project_page(
        &state.site,
        &tree,
        &project,
        theme::Theme::from_headers(&headers),
    )
    .into_response())
}

async fn blog(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    blog_disabled(&state, &headers, "/blog").await
}

async fn blog_post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    blog_disabled(&state, &headers, &format!("/blog/{slug}")).await
}

async fn sitemap(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let projects = state.db.projects().await?;
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for path in ["/", "/projects", "/about", "/profile", "/help"] {
        body.push_str(&format!(
            "  <url><loc>{}{path}</loc></url>\n",
            state.site.origin
        ));
    }
    for project in projects {
        body.push_str(&format!(
            "  <url><loc>{}/projects/{}</loc></url>\n",
            state.site.origin, project.slug
        ));
    }
    body.push_str("</urlset>\n");
    Ok((
        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
        body,
    ))
}

async fn robots(State(state): State<AppState>) -> impl IntoResponse {
    let body = format!(
        "User-agent: *\nAllow: /\n\nSitemap: {}/sitemap.xml\n",
        state.site.origin
    );
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body)
}

async fn style_sheet() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        STYLE,
    )
}

async fn profile_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let profile = state.db.profile().await?;
    let (projects, blog_enabled) = tree_data(&state).await?;
    let tree = html::Tree {
        projects: &projects,
        blog_enabled,
    };
    Ok(html::profile(
        &state.site,
        &tree,
        &profile,
        theme::Theme::from_headers(&headers),
    )
    .into_response())
}

async fn help_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (projects, blog_enabled) = tree_data(&state).await?;
    let tree = html::Tree {
        projects: &projects,
        blog_enabled,
    };
    Ok(html::help(&state.site, &tree, theme::Theme::from_headers(&headers)).into_response())
}

async fn tree_data(state: &AppState) -> Result<(Vec<hldr_core::ProjectSummary>, bool), AppError> {
    Ok((state.db.projects().await?, state.db.blog_enabled().await?))
}

async fn vim_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        VIM_JS,
    )
}

async fn keys_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        KEYS_JS,
    )
}

async fn favicon(Query(query): Query<FaviconQuery>, headers: HeaderMap) -> impl IntoResponse {
    let theme = query
        .t
        .as_deref()
        .and_then(theme::Theme::get)
        .unwrap_or_else(|| theme::Theme::from_headers(&headers));
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        theme.favicon_svg(),
    )
}

#[derive(serde::Deserialize)]
struct FaviconQuery {
    t: Option<String>,
}

async fn favicon_ico() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/vnd.microsoft.icon"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        FAVICON_ICO,
    )
}

async fn not_found(State(state): State<AppState>, uri: Uri, headers: HeaderMap) -> Response {
    render_not_found(&state, &headers, uri.path(), "path not found.").await
}

async fn blog_disabled(
    state: &AppState,
    headers: &HeaderMap,
    path: &str,
) -> Result<Response, AppError> {
    let detail = if state.db.blog_enabled().await? {
        "path not found."
    } else {
        "blog is disabled."
    };
    Ok(render_not_found(state, headers, path, detail).await)
}

async fn render_not_found(
    state: &AppState,
    headers: &HeaderMap,
    path: &str,
    detail: &str,
) -> Response {
    let projects = state.db.projects().await.unwrap_or_default();
    let blog_enabled = state.db.blog_enabled().await.unwrap_or(false);
    let tree = html::Tree {
        projects: &projects,
        blog_enabled,
    };
    let page = html::not_found(
        &state.site,
        &tree,
        path,
        detail,
        theme::Theme::from_headers(headers),
    );
    (StatusCode::NOT_FOUND, page).into_response()
}

async fn set_theme(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(theme) = theme::Theme::get(&slug) else {
        return render_not_found(
            &state,
            &headers,
            &format!("/theme/{slug}"),
            "no such theme.",
        )
        .await;
    };
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, theme::safe_return(&headers)),
            (header::SET_COOKIE, theme.cookie_header()),
        ],
    )
        .into_response()
}

async fn terminate() {
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");

    tokio::select! {
        _ = sigterm.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }

    tracing::info!("shutting down");
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    async fn state() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        let content = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
        hldr_core::index::sync(db.pool(), &content).await.unwrap();
        let site = Site {
            origin: "https://example.test".to_owned(),
            host: "example.test".to_owned(),
        };
        (dir, AppState { db, site })
    }

    async fn call(router: Router, path: &str) -> (StatusCode, String, serde_json::Value) {
        let response = router
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_owned())
            .unwrap_or_default();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, content_type, json)
    }

    #[tokio::test]
    async fn site_does_not_serve_the_api() {
        let (_dir, state) = state().await;
        for path in ["/api/v1/projects", "/api/v1/schema", "/api/v1/profile"] {
            let (status, content_type, _) = call(site_router(state.clone()), path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            assert!(
                content_type.starts_with("text/html"),
                "{path}: {content_type}"
            );
        }
    }

    #[tokio::test]
    async fn private_api_serves_resources() {
        let (_dir, state) = state().await;

        let (status, _, list) = call(api::router(state.clone()), "/api/v1/projects").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["kind"], "ProjectList");
        assert_eq!(list["items"][0]["metadata"]["slug"], "hldr");

        let (status, _, project) = call(api::router(state.clone()), "/api/v1/projects/hldr").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(project["kind"], "Project");
        assert_eq!(project["spec"]["highlight"], 1);

        let (status, _, profile) = call(api::router(state.clone()), "/api/v1/profile").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(profile["kind"], "Profile");
    }

    #[tokio::test]
    async fn private_api_serves_discovery() {
        let (_dir, state) = state().await;

        let (status, _, resources) =
            call(api::router(state.clone()), "/api/v1/api-resources").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resources["items"][0]["name"], "projects");

        let (status, _, schema) = call(api::router(state.clone()), "/api/v1/schema").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(schema["Project"]["title"], "Project");
    }

    #[tokio::test]
    async fn private_api_answers_problems() {
        let (_dir, state) = state().await;
        for path in ["/api/v1/projects/nope", "/api/v1/posts", "/nope"] {
            let (status, content_type, body) = call(api::router(state.clone()), path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            assert_eq!(content_type, "application/problem+json", "{path}");
            assert_eq!(body["status"], 404, "{path}");
        }
    }

    #[test]
    fn css_budget() {
        assert!(
            super::STYLE.len() <= 14_336,
            "style.css is {} bytes",
            super::STYLE.len()
        );
        assert!(
            super::KEYS_JS.len() <= 4_096,
            "keys.js is {} bytes",
            super::KEYS_JS.len()
        );
        assert!(
            super::VIM_JS.len() <= 40_960,
            "vim.js is {} bytes",
            super::VIM_JS.len()
        );
    }
}
