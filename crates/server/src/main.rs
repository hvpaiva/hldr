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
mod negotiate;
mod text;
mod theme;

const DEFAULT_ADDR: &str = "127.0.0.1:8080";
const STYLE: &str = include_str!("../assets/style.css");
const KEYS_JS: &str = include_str!("../assets/keys.js");
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
    let app = router(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind HLDR_ADDR");

    tracing::info!(%addr, version = hldr_core::VERSION, "hldr-server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(terminate())
        .await
        .expect("server error");
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/projects", get(projects))
        .route("/projects.txt", get(projects_txt))
        .route("/projects/{slug}", get(project))
        .route("/about", get(about))
        .route("/about.txt", get(about_txt))
        .route("/blog", get(blog))
        .route("/blog/{slug}", get(blog_post))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/theme", get(themes))
        .route("/theme.txt", get(themes_txt))
        .route("/theme/{slug}", get(set_theme))
        .route("/style.css", get(style_sheet))
        .route("/keys.js", get(keys_js))
        .route("/favicon.svg", get(favicon))
        .route("/favicon.ico", get(favicon_ico))
        .route("/sitemap.xml", get(sitemap))
        .route("/robots.txt", get(robots))
        .route("/api/v1/projects", get(api_projects))
        .route("/api/v1/projects/{slug}", get(api_project))
        .route("/api/v1/profile", get(api_profile))
        .route("/api/v1/posts", get(api_posts))
        .route("/api/v1/posts/{slug}", get(api_post))
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
            .unwrap_or_else(|_| "https://hvpaiva.dev".to_owned())
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
    if let Ok(path) = std::env::var("HLDR_DATABASE") {
        return PathBuf::from(path);
    }
    let dir = std::env::var("STATE_DIRECTORY").unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(dir).join("hldr.db")
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
                Json(serde_json::json!({ "status": "unready", "version": hldr_core::VERSION })),
            )
                .into_response()
        }
    }
}

async fn home(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    render_home(state, headers, false).await
}

async fn projects(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    render_projects(state, headers, false).await
}

async fn projects_txt(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    render_projects(state, headers, true).await
}

async fn about(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    render_about(state, headers, false).await
}

async fn about_txt(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    render_about(state, headers, true).await
}

async fn themes(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    render_themes(state, headers, false).await
}

async fn themes_txt(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    render_themes(state, headers, true).await
}

async fn project(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (slug, force_txt) = negotiate::strip_txt(&raw);
    let Some(project) = state.db.project(slug).await? else {
        return Ok(render_not_found(
            &state,
            &headers,
            &format!("/projects/{raw}"),
            force_txt,
            &format!("{slug}: no such project"),
        )
        .await);
    };
    let count = state.db.project_count().await?;
    if negotiate::wants_text(&headers, force_txt) {
        Ok(plain(text::project_page(
            &state.site,
            &project,
            negotiate::wants_color(&headers, force_txt),
        )))
    } else {
        Ok(html_page(html::project_page(
            &state.site,
            &project,
            count,
            theme::Theme::from_headers(&headers),
        )))
    }
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

async fn api_projects(State(state): State<AppState>) -> Result<Response, AppError> {
    let items = state.db.projects().await?;
    Ok(api::projects(&items).into_response())
}

async fn api_project(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, AppError> {
    let Some(project) = state.db.project(&slug).await? else {
        return Ok(api::not_found(format!("project '{slug}' not found")));
    };
    Ok(api::project(&project).into_response())
}

async fn api_profile(State(state): State<AppState>) -> Result<Response, AppError> {
    let profile = state.db.profile().await?;
    Ok(api::profile(&profile).into_response())
}

async fn api_posts(State(state): State<AppState>) -> Result<Response, AppError> {
    if state.db.blog_enabled().await? {
        return Ok(api::not_found("no posts"));
    }
    Ok(api::not_found("blog is disabled"))
}

async fn api_post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, AppError> {
    if state.db.blog_enabled().await? {
        return Ok(api::not_found(format!("post '{slug}' not found")));
    }
    Ok(api::not_found("blog is disabled"))
}

async fn sitemap(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let projects = state.db.projects().await?;
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for path in ["/", "/projects", "/about"] {
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
    let path = uri.path();
    let (_, force_txt) = negotiate::strip_txt(path);
    render_not_found(&state, &headers, path, force_txt, "path not found.").await
}

async fn render_home(
    state: AppState,
    headers: HeaderMap,
    force_txt: bool,
) -> Result<Response, AppError> {
    let profile = state.db.profile().await?;
    let projects = state.db.highlighted_projects().await?;
    let count = state.db.project_count().await?;
    if negotiate::wants_text(&headers, force_txt) {
        Ok(plain(text::home(
            &state.site,
            &profile,
            &projects,
            negotiate::wants_color(&headers, force_txt),
        )))
    } else {
        Ok(html_page(html::home(
            &state.site,
            &profile,
            &projects,
            count,
            theme::Theme::from_headers(&headers),
        )))
    }
}

async fn render_projects(
    state: AppState,
    headers: HeaderMap,
    force_txt: bool,
) -> Result<Response, AppError> {
    let projects = state.db.projects().await?;
    let count = state.db.project_count().await?;
    if negotiate::wants_text(&headers, force_txt) {
        Ok(plain(text::projects_index(
            &state.site,
            &projects,
            negotiate::wants_color(&headers, force_txt),
        )))
    } else {
        Ok(html_page(html::projects_index(
            &state.site,
            &projects,
            count,
            "Work worth opening. Context lives on the project page, not the README.",
            theme::Theme::from_headers(&headers),
        )))
    }
}

async fn render_about(
    state: AppState,
    headers: HeaderMap,
    force_txt: bool,
) -> Result<Response, AppError> {
    let profile = state.db.profile().await?;
    let count = state.db.project_count().await?;
    if negotiate::wants_text(&headers, force_txt) {
        Ok(plain(text::about(
            &state.site,
            &profile,
            negotiate::wants_color(&headers, force_txt),
        )))
    } else {
        Ok(html_page(html::about(
            &state.site,
            &profile,
            count,
            theme::Theme::from_headers(&headers),
        )))
    }
}

async fn render_themes(
    state: AppState,
    headers: HeaderMap,
    force_txt: bool,
) -> Result<Response, AppError> {
    let theme = theme::Theme::from_headers(&headers);
    let profile = state.db.profile().await?;
    let projects = state.db.highlighted_projects().await?;
    let count = state.db.project_count().await?;
    if negotiate::wants_text(&headers, force_txt) {
        Ok(plain(text::themes_index(
            &state.site,
            theme,
            negotiate::wants_color(&headers, force_txt),
        )))
    } else {
        Ok(html_page(html::themes_index(
            &state.site,
            &profile,
            &projects,
            count,
            theme,
        )))
    }
}

async fn blog_disabled(
    state: &AppState,
    headers: &HeaderMap,
    path: &str,
) -> Result<Response, AppError> {
    if state.db.blog_enabled().await? {
        return Ok(render_not_found(state, headers, path, false, "path not found.").await);
    }
    Ok(render_not_found(state, headers, path, false, "blog is disabled.").await)
}

async fn render_not_found(
    state: &AppState,
    headers: &HeaderMap,
    path: &str,
    force_txt: bool,
    detail: &str,
) -> Response {
    let count = state.db.project_count().await.unwrap_or(0);
    if negotiate::wants_text(headers, force_txt) {
        (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8"), vary()],
            text::not_found(
                &state.site,
                negotiate::wants_color(headers, force_txt),
                detail,
            ),
        )
            .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            [vary()],
            html::not_found(
                &state.site,
                path,
                count,
                detail,
                theme::Theme::from_headers(headers),
            ),
        )
            .into_response()
    }
}

fn html_page(markup: maud::Markup) -> Response {
    ([vary()], markup).into_response()
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
            false,
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

fn plain(body: String) -> Response {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8"), vary()],
        body,
    )
        .into_response()
}

fn vary() -> (header::HeaderName, &'static str) {
    (header::VARY, "Accept, User-Agent")
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
    }
}
