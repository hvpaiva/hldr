use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

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
mod content;
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
    syncer: Arc<content::Syncer>,
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
    let source = content::ContentSource::from_env().unwrap_or_else(|err| panic!("{err}"));
    let poll = content::poll_interval().unwrap_or_else(|err| panic!("{err}"));

    let db = Db::open(&db_path).await.expect("failed to open database");
    let syncer = Arc::new(
        content::Syncer::new(source, db.clone())
            .with_token(std::env::var("HLDR_GITHUB_TOKEN").ok()),
    );
    // A failed first sync still serves whatever revision the database holds;
    // with none, /readyz stays unready and the deploy never takes traffic.
    match syncer.sync(true).await {
        Ok(report) => tracing::info!(
            source = %syncer.source().describe(),
            db = %db_path.display(),
            report = ?report,
            "content synced"
        ),
        Err(error) => tracing::error!(
            source = %syncer.source().describe(),
            %error,
            "content sync failed"
        ),
    }
    if let Some(every) = poll {
        tokio::spawn(poll_content(Arc::clone(&syncer), every));
    }

    let state = AppState {
        db,
        site: Site::from_env(),
        syncer,
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
        .route("/health", get(health_page))
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

async fn poll_content(syncer: Arc<content::Syncer>, every: std::time::Duration) {
    let mut ticker = tokio::time::interval(every);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;
    loop {
        ticker.tick().await;
        match syncer.sync(false).await {
            Ok(Some(report)) => tracing::info!(report = ?report, "content synced"),
            Ok(None) => tracing::debug!("content current"),
            Err(error) => tracing::warn!(%error, "content sync failed"),
        }
    }
}

/// Liveness: never touches the database.
async fn healthz() -> Json<Health> {
    Json(Health::ok(None))
}

/// Ready once this binary has materialized some revision of the content.
/// A database synced only by an older release has no `site` row yet, so a
/// deploy that cannot sync keeps the previous container serving.
async fn readyz(State(state): State<AppState>) -> Response {
    let synced = async {
        state.db.ping().await?;
        let sync = state.db.sync_state().await?;
        let site = state.db.site().await?;
        Ok::<_, hldr_core::Error>((sync, site.is_some()))
    };
    match synced.await {
        Ok((sync, true)) if sync.synced_at.is_some() => {
            Json(Health::ok(sync.revision)).into_response()
        }
        Ok(_) => unready("content not synced by this release"),
        Err(error) => unready(&error.to_string()),
    }
}

fn unready(reason: &str) -> Response {
    tracing::error!(reason, "readyz failed");
    (StatusCode::SERVICE_UNAVAILABLE, Json(Health::unready(None))).into_response()
}

/// What every page draws around its buffer, loaded once per request.
struct Chrome {
    projects: Vec<hldr_core::ProjectSummary>,
    site: hldr_core::SiteConfig,
    profile: hldr_core::Profile,
    theme: hldr_core::Theme,
}

impl Chrome {
    async fn load(state: &AppState, headers: &HeaderMap) -> Result<Self, AppError> {
        let site = state.db.site_config().await?;
        let theme = current_theme(&state.db, headers, &site).await?;
        Ok(Self {
            projects: state.db.projects().await?,
            profile: state.db.profile().await?,
            site,
            theme,
        })
    }

    fn tree(&self) -> html::Tree<'_> {
        html::Tree {
            projects: &self.projects,
            site: &self.site,
            profile: &self.profile,
        }
    }
}

/// The visitor's pick while that theme still exists, else the site's default.
async fn current_theme(
    db: &Db,
    headers: &HeaderMap,
    site: &hldr_core::SiteConfig,
) -> Result<hldr_core::Theme, AppError> {
    if let Some(slug) = theme::cookie_slug(headers)
        && let Some(picked) = db.theme(slug).await?
    {
        return Ok(picked);
    }
    Ok(db
        .theme(&site.theme)
        .await?
        .ok_or(hldr_core::Error::Invariant("default theme missing"))?)
}

async fn home(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let chrome = Chrome::load(&state, &headers).await?;
    let highlighted = state.db.highlighted_projects().await?;
    Ok(html::home(
        &state.site,
        &chrome.tree(),
        &chrome.profile,
        &highlighted,
        &chrome.theme,
    )
    .into_response())
}

async fn projects(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let chrome = Chrome::load(&state, &headers).await?;
    Ok(html::projects_index(&state.site, &chrome.tree(), &chrome.theme).into_response())
}

async fn about(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let chrome = Chrome::load(&state, &headers).await?;
    Ok(html::about(&state.site, &chrome.tree(), &chrome.profile, &chrome.theme).into_response())
}

async fn themes(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let chrome = Chrome::load(&state, &headers).await?;
    let themes = state.db.themes().await?;
    Ok(html::themes_index(&state.site, &chrome.tree(), &themes, &chrome.theme).into_response())
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
    let chrome = Chrome::load(&state, &headers).await?;
    Ok(html::project_page(&state.site, &chrome.tree(), &project, &chrome.theme).into_response())
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
    let chrome = Chrome::load(&state, &headers).await?;
    Ok(html::profile(&state.site, &chrome.tree(), &chrome.profile, &chrome.theme).into_response())
}

async fn help_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let chrome = Chrome::load(&state, &headers).await?;
    Ok(html::help(&state.site, &chrome.tree(), &chrome.theme).into_response())
}

async fn health_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let chrome = Chrome::load(&state, &headers).await?;
    let sync = state.db.sync_state().await?;
    let themes = state.db.themes().await?.len();
    let origin = state.syncer.source().origin();
    let report = html::Health {
        source: state.syncer.source().describe(),
        repository: origin.as_ref().map(|(repo, _)| repo.as_str()),
        sync: &sync,
        projects: state.db.project_count().await?,
        themes,
    };
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        html::health(&state.site, &chrome.tree(), &report, &chrome.theme),
    )
        .into_response())
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

async fn favicon(
    State(state): State<AppState>,
    Query(query): Query<FaviconQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let asked = match query.t.as_deref() {
        Some(slug) => state.db.theme(slug).await?,
        None => None,
    };
    let theme = match asked {
        Some(theme) => theme,
        None => current_theme(&state.db, &headers, &state.db.site_config().await?).await?,
    };
    Ok((
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        theme::favicon_svg(&theme),
    )
        .into_response())
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
    match Chrome::load(state, headers).await {
        Ok(chrome) => {
            let page = html::not_found(&state.site, &chrome.tree(), path, detail, &chrome.theme);
            (StatusCode::NOT_FOUND, page).into_response()
        }
        Err(error) => error.into_response(),
    }
}

async fn set_theme(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(theme) = state.db.theme(&slug).await? else {
        return Ok(render_not_found(
            &state,
            &headers,
            &format!("/theme/{slug}"),
            "no such theme.",
        )
        .await);
    };
    Ok((
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, theme::safe_return(&headers)),
            (header::SET_COOKIE, theme::cookie_header(&theme)),
        ],
    )
        .into_response())
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

    fn fixtures() -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/content")
    }

    /// State over a content directory, synced unless `synced` is false.
    async fn state_over(content: PathBuf, synced: bool) -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        let syncer = Arc::new(content::Syncer::new(
            content::ContentSource::Dir(content),
            db.clone(),
        ));
        if synced {
            syncer.sync(true).await.unwrap();
        }
        let site = Site {
            origin: "https://example.test".to_owned(),
            host: "example.test".to_owned(),
        };
        (dir, AppState { db, site, syncer })
    }

    async fn state() -> (tempfile::TempDir, AppState) {
        state_over(fixtures(), true).await
    }

    /// A writable copy of the fixtures, for tests that change content.
    fn fixture_copy() -> tempfile::TempDir {
        fn copy(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &target);
                } else {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        copy(&fixtures(), dir.path());
        dir
    }

    async fn call(router: Router, path: &str) -> (StatusCode, String, serde_json::Value) {
        send(router, Request::get(path).body(Body::empty()).unwrap()).await
    }

    async fn post(router: Router, path: &str) -> (StatusCode, String, serde_json::Value) {
        send(router, Request::post(path).body(Body::empty()).unwrap()).await
    }

    async fn send(
        router: Router,
        request: Request<Body>,
    ) -> (StatusCode, String, serde_json::Value) {
        let response = router.oneshot(request).await.unwrap();
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
        for path in [
            "/api/v1/projects",
            "/api/v1/schema",
            "/api/v1/profile",
            "/api/v1/site",
        ] {
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
        assert_eq!(list["items"][0]["metadata"]["name"], "atlas");
        assert!(
            list["items"][0]["spec"]["body"]
                .as_str()
                .unwrap()
                .contains("What it is")
        );

        let (status, _, project) = call(api::router(state.clone()), "/api/v1/projects/atlas").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(project["kind"], "Project");
        assert_eq!(project["spec"]["highlight"], 1);

        let (status, _, profile) = call(api::router(state.clone()), "/api/v1/profile").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(profile["kind"], "Profile");

        let (status, _, site) = call(api::router(state.clone()), "/api/v1/site").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(site["kind"], "Site");
        assert_eq!(site["spec"]["blog"]["enabled"], false);
    }

    #[tokio::test]
    async fn drafts_reach_the_api_but_not_the_site() {
        let (_dir, state) = state().await;

        let (_, _, list) = call(api::router(state.clone()), "/api/v1/projects").await;
        let names: Vec<&str> = list["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["metadata"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["atlas", "sketch"]);

        let (status, _, draft) = call(api::router(state.clone()), "/api/v1/projects/sketch").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(draft["spec"]["draft"], true);

        let (status, _, _) = call(site_router(state.clone()), "/projects/sketch").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        for path in ["/", "/projects", "/sitemap.xml"] {
            let response = site_router(state.clone())
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let page = String::from_utf8_lossy(&bytes);
            assert!(page.contains("atlas"), "{path}");
            assert!(!page.contains("sketch"), "{path}");
        }
    }

    #[tokio::test]
    async fn private_api_reports_and_runs_syncs() {
        let (_dir, state) = state().await;

        let (status, _, sync) = call(api::router(state.clone()), "/api/v1/sync").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(sync["kind"], "SyncStatus");
        assert!(sync["source"].as_str().unwrap().starts_with("dir:"));
        assert!(sync["synced_at"].is_string());
        assert!(sync["revision"].is_null());
        assert!(sync.get("report").is_none());

        let (status, _, sync) = post(api::router(state.clone()), "/api/v1/sync").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(sync["report"]["projects_skipped"], 2);
        assert!(sync["repository"].is_null());

        let request = Request::post("/api/v1/sync")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"revision":"nope"}"#))
            .unwrap();
        let (status, content_type, body) = send(api::router(state.clone()), request).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(content_type, "application/problem+json");
        assert!(body["detail"].as_str().unwrap().contains("not a commit id"));
    }

    #[tokio::test]
    async fn invalid_content_keeps_the_previous_revision() {
        let content = fixture_copy();
        let (_dir, state) = state_over(content.path().to_owned(), true).await;
        std::fs::write(
            content.path().join("projects/atlas.md"),
            "---
kind: Site
---
",
        )
        .unwrap();

        let (status, content_type, body) = post(api::router(state.clone()), "/api/v1/sync").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(content_type, "application/problem+json");
        assert!(
            body["detail"]
                .as_str()
                .unwrap()
                .contains("projects/atlas.md"),
            "{body}"
        );

        let (_, _, project) = call(api::router(state.clone()), "/api/v1/projects/atlas").await;
        assert_eq!(project["spec"]["title"], "Atlas");
        let (_, _, sync) = call(api::router(state.clone()), "/api/v1/sync").await;
        assert!(
            sync["last_error"]
                .as_str()
                .unwrap()
                .contains("projects/atlas.md")
        );
    }

    #[tokio::test]
    async fn ready_only_once_content_is_synced() {
        let (_dir, state) = state_over(fixtures(), false).await;
        let (status, _, body) = call(site_router(state.clone()), "/readyz").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], "unready");

        state.syncer.sync(true).await.unwrap();
        let (status, _, body) = call(site_router(state.clone()), "/readyz").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
    }

    #[tokio::test]
    async fn api_failures_are_problems() {
        let (_dir, state) = state().await;
        state.db.pool().close().await;
        let (status, content_type, body) =
            call(api::router(state.clone()), "/api/v1/projects").await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(content_type, "application/problem+json");
        assert_eq!(body["detail"], "internal error");
    }

    async fn page(router: Router, path: &str, cookie: Option<&str>) -> (StatusCode, String) {
        let mut request = Request::get(path);
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let response = router
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn pages_take_their_identity_from_content() {
        let (_dir, state) = state().await;
        for path in ["/", "/projects", "/about", "/profile", "/theme"] {
            let (status, body) = page(site_router(state.clone()), path, None).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert!(body.contains("example.test"), "{path}");
            assert!(!body.contains("hvpaiva"), "{path}");
            assert!(
                body.contains("https://github.com/example"),
                "{path}: elsewhere"
            );
        }
        let (_, home) = page(site_router(state.clone()), "/", None).await;
        assert!(home.contains("AMPLE"), "banner");
        let (_, projects) = page(site_router(state.clone()), "/projects", None).await;
        assert!(projects.contains("Fixture projects."));
    }

    #[tokio::test]
    async fn health_is_a_page() {
        let content = fixture_copy();
        let (_dir, state) = state_over(content.path().to_owned(), true).await;
        let response = site_router(state.clone())
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");

        let (_, body) = page(site_router(state.clone()), "/health", None).await;
        assert!(body.contains("- OK</span> source dir:"), "{body}");
        assert!(body.contains("serving the directory as it is on disk"));
        assert!(body.contains("2 projects and 2 themes indexed"));
        assert!(body.contains("succeeded"));
        assert!(!body.contains("WARNING"));
        assert!(!body.contains("<script>"), "no inline script");

        std::fs::write(
            content.path().join("projects/atlas.md"),
            "---\nkind: Site\n---\n",
        )
        .unwrap();
        assert!(state.syncer.sync(false).await.is_err());
        let (_, body) = page(site_router(state.clone()), "/health", None).await;
        assert!(
            body.contains("- WARNING</span> last sync attempt"),
            "{body}"
        );
        assert!(body.contains("projects/atlas.md"));

        let (_, home) = page(site_router(state.clone()), "/", None).await;
        assert!(
            home.contains(r#"<a href="/health" title=":checkhealth">hldr "#),
            "statusline"
        );
        assert!(
            home.contains(r#"href="/health" data-cmd="checkhealth""#),
            "tree"
        );
    }

    #[tokio::test]
    async fn the_theme_cookie_picks_among_content_themes() {
        let (_dir, state) = state().await;
        let (_, default) = page(site_router(state.clone()), "/", None).await;
        assert!(default.contains("--bg:#2e3440"));
        let (_, picked) = page(site_router(state.clone()), "/", Some("theme=dawn")).await;
        assert!(picked.contains("--bg:#faf4ed"));
        assert!(picked.contains("color-scheme:light"));
        let (_, unknown) = page(site_router(state.clone()), "/", Some("theme=gone")).await;
        assert!(unknown.contains("--bg:#2e3440"));

        let (_, picker) = page(site_router(state.clone()), "/theme", None).await;
        assert!(picker.contains("2 palettes"));
        assert!(picker.contains("/theme/dawn"));

        let response = site_router(state.clone())
            .oneshot(Request::get("/theme/dawn").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.starts_with("theme=dawn;"), "{cookie}");

        let (status, _) = page(site_router(state.clone()), "/theme/gone", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn private_api_serves_themes() {
        let (_dir, state) = state().await;
        let (status, _, list) = call(api::router(state.clone()), "/api/v1/themes").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["kind"], "ThemeList");
        assert_eq!(list["items"][0]["metadata"]["name"], "dawn");
        let (status, _, nord) = call(api::router(state.clone()), "/api/v1/themes/nord").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(nord["spec"]["colors"]["accent"], "#88c0d0");
        let (status, _, _) = call(api::router(state.clone()), "/api/v1/themes/gone").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (_, _, site) = call(api::router(state.clone()), "/api/v1/site").await;
        assert_eq!(site["spec"]["theme"], "nord");
        assert_eq!(site["spec"]["descriptions"]["themes"], "Fixture palettes.");
    }

    #[tokio::test]
    async fn private_api_serves_discovery() {
        let (_dir, state) = state().await;

        let (status, _, resources) =
            call(api::router(state.clone()), "/api/v1/api-resources").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resources["items"][0]["name"], "projects");
        assert_eq!(
            resources["items"][0]["source"]["path"],
            "projects/{name}.md"
        );

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
