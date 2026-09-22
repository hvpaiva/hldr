//! Counting the pages the site serves, without tracking anyone: see
//! [`hldr_core::metrics`]. Requests are counted into memory as they are
//! answered, and flushed to the database every minute and on shutdown.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use hldr_core::Db;
use hldr_core::metrics::{self, Batch, NOT_FOUND};

/// How often counts reach the database.
pub const FLUSH_EVERY: Duration = Duration::from_secs(60);

/// Distinct referring hosts kept between flushes: `Referer` is whatever the
/// client sends, so a flood of made-up hosts must not grow memory.
const MAX_REFERRERS: usize = 1024;

/// Visitor hashes kept between flushes, for the same reason: about 8 MiB.
const MAX_VISITORS: usize = 100_000;

/// User agents that name themselves as tools or crawlers, lowercased. A
/// request without a user agent is a bot too.
const BOTS: &[&str] = &[
    "bot",
    "crawl",
    "spider",
    "slurp",
    "curl",
    "wget",
    "python-requests",
    "python-urllib",
    "go-http-client",
    "headless",
    "httpclient",
    "java/",
    "libwww",
    "okhttp",
    "axios",
    "node-fetch",
    "scrapy",
    "facebookexternalhit",
    "preview",
    "monitor",
];

/// What a server counted and not yet flushed, plus the salt of the day.
pub struct Metrics {
    db: Db,
    /// The site's own host, whose links are not referrals.
    host: String,
    batch: Mutex<Batch>,
    salt: Mutex<Option<(String, Arc<[u8]>)>>,
}

/// A request worth counting. The address and user agent live only until
/// they are hashed.
struct Visit {
    day: String,
    route: String,
    address: String,
    user_agent: Option<String>,
    referrer: Option<String>,
}

impl Metrics {
    /// `host` is the site's own, as `HLDR_ORIGIN` names it.
    pub fn new(db: Db, host: &str) -> Self {
        Self {
            db,
            host: own_host(host),
            batch: Mutex::default(),
            salt: Mutex::default(),
        }
    }

    async fn count(&self, visit: Visit) {
        if is_bot(visit.user_agent.as_deref()) {
            *lock(&self.batch).bots.entry(visit.day).or_default() += 1;
            return;
        }
        let hash = match self.salt(&visit.day).await {
            Ok(salt) => Some(metrics::visitor(
                &salt,
                &visit.address,
                visit.user_agent.as_deref().unwrap_or_default(),
            )),
            Err(error) => {
                tracing::warn!(%error, "no salt for the day; this view counts no visitor");
                None
            }
        };
        let mut batch = lock(&self.batch);
        if let Some(host) = visit.referrer
            && (batch.referrers.len() < MAX_REFERRERS
                || batch
                    .referrers
                    .contains_key(&(visit.day.clone(), host.clone())))
        {
            *batch
                .referrers
                .entry((visit.day.clone(), host))
                .or_default() += 1;
        }
        if let Some(hash) = hash
            && batch.visitors.len() < MAX_VISITORS
        {
            batch
                .visitors
                .insert((visit.day.clone(), visit.route.clone(), hash));
        }
        *batch.views.entry((visit.day, visit.route)).or_default() += 1;
    }

    /// The day's salt, read once a day from the database, where the first
    /// server to need it creates it.
    async fn salt(&self, day: &str) -> Result<Arc<[u8]>, hldr_core::Error> {
        if let Some((cached, salt)) = lock(&self.salt).as_ref()
            && cached == day
        {
            return Ok(Arc::clone(salt));
        }
        let salt: Arc<[u8]> = self.db.salt(day).await?.into();
        *lock(&self.salt) = Some((day.to_owned(), Arc::clone(&salt)));
        Ok(salt)
    }

    /// Writes what was counted, and closes the days that ended. A failed
    /// write keeps the counts for the next flush.
    pub async fn flush(&self) {
        let batch = std::mem::take(&mut *lock(&self.batch));
        match self.db.flush_metrics(&batch).await {
            Ok(closed) => {
                for day in closed {
                    tracing::info!(%day, "metrics day closed");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "metrics flush failed; kept for the next one");
                lock(&self.batch).merge(batch);
            }
        }
    }
}

/// A host as referrers are compared with it: lowercased, without a port.
fn own_host(host: &str) -> String {
    host.split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// A poisoned lock still holds counts worth keeping.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Flushes every `every`, for as long as the server runs.
pub async fn flush_every(metrics: Arc<Metrics>, every: Duration) {
    let mut ticker = tokio::time::interval(every);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;
    loop {
        ticker.tick().await;
        metrics.flush().await;
    }
}

/// Middleware of the site: counts every page served and every 404, after
/// the response is built and without touching it.
pub async fn count(State(metrics): State<Arc<Metrics>>, request: Request, next: Next) -> Response {
    if request.method() != Method::GET {
        return next.run(request).await;
    }
    let day = metrics::today();
    let path = request.uri().path().to_owned();
    let headers = request.headers();
    let socket = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(socket)| socket.ip());
    let address = client_address(headers, socket);
    let user_agent = text(headers, header::USER_AGENT).map(str::to_owned);
    let referrer =
        text(headers, header::REFERER).and_then(|referer| referrer_host(referer, &metrics.host));

    let response = next.run(request).await;
    let content_type = text(response.headers(), header::CONTENT_TYPE);
    if let Some(route) = route(&path, response.status(), content_type) {
        metrics
            .count(Visit {
                day,
                route: route.to_owned(),
                address,
                user_agent,
                referrer,
            })
            .await;
    }
    response
}

fn text(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// The route a response counts under: its path for a page, the one `404`
/// bucket for anything not found, nothing for the rest.
fn route<'a>(path: &'a str, status: StatusCode, content_type: Option<&str>) -> Option<&'a str> {
    if path.starts_with("/theme/") {
        return None;
    }
    let html = content_type.is_some_and(|content_type| content_type.starts_with("text/html"));
    match status {
        StatusCode::OK if html => Some(path),
        StatusCode::NOT_FOUND => Some(NOT_FOUND),
        _ => None,
    }
}

/// The client's address. kamal-proxy terminates TLS and sets
/// `X-Forwarded-For` to the address it saw; were a client's own header ever
/// passed along, the proxy's entry would still be the last one, so only the
/// last is trusted.
fn client_address(headers: &HeaderMap, socket: Option<IpAddr>) -> String {
    headers
        .get_all("x-forwarded-for")
        .iter()
        .next_back()
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit(',').next())
        .and_then(|entry| entry.trim().parse::<IpAddr>().ok())
        .or(socket)
        .map(|address| address.to_string())
        .unwrap_or_default()
}

fn is_bot(user_agent: Option<&str>) -> bool {
    let Some(user_agent) = user_agent.filter(|agent| !agent.trim().is_empty()) else {
        return true;
    };
    let user_agent = user_agent.to_ascii_lowercase();
    BOTS.iter().any(|bot| user_agent.contains(bot))
}

/// The host of an `http` or `https` referrer, lowercased; none for the site
/// itself or for anything that is not a plain host name.
fn referrer_host(referer: &str, own: &str) -> Option<String> {
    let referer = referer.trim().to_ascii_lowercase();
    let rest = referer
        .strip_prefix("https://")
        .or_else(|| referer.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    let host = host.trim_end_matches('.');
    let plain = !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-');
    (plain && host != own).then(|| host.to_owned())
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn counts_pages_and_404s_only() {
        let html = Some("text/html; charset=utf-8");
        assert_eq!(route("/", StatusCode::OK, html), Some("/"));
        assert_eq!(
            route("/projects/atlas", StatusCode::OK, html),
            Some("/projects/atlas")
        );
        assert_eq!(
            route("/wp-login.php", StatusCode::NOT_FOUND, html),
            Some("404")
        );
        assert_eq!(route("/style.css", StatusCode::OK, Some("text/css")), None);
        assert_eq!(
            route("/healthz", StatusCode::OK, Some("application/json")),
            None
        );
        assert_eq!(route("/", StatusCode::INTERNAL_SERVER_ERROR, html), None);
        assert_eq!(route("/theme/nord", StatusCode::SEE_OTHER, None), None);
        assert_eq!(route("/theme/nope", StatusCode::NOT_FOUND, html), None);
    }

    #[test]
    fn the_client_is_the_last_forwarded_address() {
        let socket = Some("10.0.0.9".parse().unwrap());
        let mut headers = HeaderMap::new();
        assert_eq!(client_address(&headers, socket), "10.0.0.9");
        assert_eq!(client_address(&headers, None), "");
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));
        assert_eq!(client_address(&headers, socket), "203.0.113.7");
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("junk, 1.1.1.1, 2001:db8::1 "),
        );
        assert_eq!(client_address(&headers, socket), "2001:db8::1");
        headers.append("x-forwarded-for", HeaderValue::from_static("192.0.2.4"));
        assert_eq!(client_address(&headers, socket), "192.0.2.4");
        headers.insert("x-forwarded-for", HeaderValue::from_static("1.1.1.1, junk"));
        assert_eq!(
            client_address(&headers, socket),
            "10.0.0.9",
            "an entry before the last is never trusted"
        );
        assert_eq!(client_address(&headers, socket), "10.0.0.9");
    }

    #[test]
    fn bots_name_themselves() {
        for bot in [
            None,
            Some(""),
            Some("Mozilla/5.0 (compatible; Googlebot/2.1)"),
            Some("curl/8.9.1"),
            Some("Go-http-client/2.0"),
            Some("Mozilla/5.0 HeadlessChrome/120.0"),
        ] {
            assert!(is_bot(bot), "{bot:?}");
        }
        assert!(!is_bot(Some(
            "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0"
        )));
    }

    #[test]
    fn a_referrer_is_a_foreign_host() {
        let host = |referer| referrer_host(referer, "hvpaiva.dev");
        assert_eq!(
            host("https://News.YCombinator.com/item?id=1").as_deref(),
            Some("news.ycombinator.com")
        );
        assert_eq!(
            host("http://user:pw@example.org:8080/x").as_deref(),
            Some("example.org")
        );
        assert_eq!(host("https://example.org.").as_deref(), Some("example.org"));
        assert_eq!(host("https://hvpaiva.dev/projects"), None);
        assert_eq!(host("android-app://com.google.android.gm/"), None);
        assert_eq!(host("https://[::1]/"), None);
        assert_eq!(host("https://exa<script>mple.org/"), None);
        assert_eq!(host("https:///path"), None);
        assert_eq!(own_host("Example.TEST:8080"), "example.test");
    }
}
