//! A local stand-in for the private API, for tests that need real HTTP.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use serde_json::{Value, json};

#[derive(Clone, Default)]
pub struct Stub {
    pub discovery_calls: Arc<AtomicUsize>,
    /// Kinds served beyond `projects`, added while a test runs.
    pub extra: Arc<std::sync::Mutex<Vec<Value>>>,
    /// What `/healthz` reports as the server's version.
    pub version: Arc<std::sync::Mutex<String>>,
    /// What `/api/v1/sync` reports as the served revision.
    pub revision: Arc<std::sync::Mutex<String>>,
}

pub fn resource(name: &str, singular: &str, kind: &str) -> Value {
    json!({
        "name": name, "singular": singular, "short_names": [], "kind": kind,
        "singleton": false, "verbs": ["get", "describe", "explain"],
        "source": {"path": format!("{name}/{{name}}.yaml")},
        "columns": [{"name": "NAME", "json_path": ".metadata.name", "wide": false}],
    })
}

/// Serves `app` on a random local port from a background thread and returns
/// the base URL.
pub fn spawn(app: Router) -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    format!("http://{}", rx.recv().unwrap())
}

impl Stub {
    pub fn calls(&self) -> usize {
        self.discovery_calls.load(Ordering::SeqCst)
    }

    pub fn set_version(&self, version: &str) {
        *self.version.lock().unwrap() = version.to_owned();
    }

    pub fn set_revision(&self, revision: &str) {
        *self.revision.lock().unwrap() = revision.to_owned();
    }

    /// Serves the stub; see [`spawn`].
    pub fn serve(&self) -> String {
        let stub = self.clone();
        let version = Arc::clone(&self.version);
        let revision = Arc::clone(&self.revision);
        let app = Router::new()
            .route(
                "/healthz",
                get(move || {
                    let version = version.lock().unwrap().clone();
                    async move { axum::Json(json!({"status": "ok", "version": version})) }
                }),
            )
            .route(
                "/api/v1/sync",
                get(move || {
                    let revision = revision.lock().unwrap().clone();
                    async move {
                        axum::Json(json!({"kind": "SyncStatus", "source": "stub", "revision": revision}))
                    }
                }),
            )
            .route(
                "/api/v1/api-resources",
                get(move || {
                    let stub = stub.clone();
                    async move {
                        stub.discovery_calls.fetch_add(1, Ordering::SeqCst);
                        let mut items = vec![resource("projects", "project", "Project")];
                        items.extend(stub.extra.lock().unwrap().iter().cloned());
                        axum::Json(json!({"kind": "APIResourceList", "items": items}))
                    }
                }),
            )
            .route("/api/v1/schema", get(|| async { axum::Json(json!({"Project": {}})) }))
            .route(
                "/api/v1/projects/atlas",
                get(|| async { axum::Json(json!({"kind": "Project", "metadata": {"name": "atlas"}})) }),
            )
            .route(
                "/api/v1/broken",
                get(|| async {
                    (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        [(header::CONTENT_TYPE, "application/problem+json")],
                        r#"{"type":"about:blank","title":"Unprocessable Content","status":422,"detail":"content is invalid"}"#,
                    )
                        .into_response()
                }),
            )
            .route(
                "/api/v1/plain",
                get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response() }),
            );
        spawn(app)
    }
}

/// A GitHub repository in memory, behind the endpoints the CLI calls, plus
/// the server's sync endpoints: enough to run a write from start to end.
pub mod hub {
    use std::collections::{BTreeMap, HashMap};
    use std::sync::{Arc, Mutex};

    use axum::Json;
    use axum::Router;
    use axum::extract::{Path, Query, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, patch, post};
    use serde_json::{Value, json};

    use super::spawn;

    type Files = BTreeMap<String, String>;

    #[derive(Default)]
    pub struct Repo {
        commits: HashMap<String, (String, Option<String>, String)>,
        trees: HashMap<String, Files>,
        pub head: String,
        next: u64,
        /// Revisions the CLI asked the server to sync.
        pub synced: Vec<Option<String>>,
    }

    impl Repo {
        fn id(&mut self) -> String {
            self.next += 1;
            format!("{:040x}", self.next)
        }

        /// Commits `files` directly, as a push from elsewhere would.
        pub fn push(&mut self, files: &[(&str, &str)], message: &str) -> String {
            let mut tree = self.files(&self.head.clone());
            for (path, text) in files {
                tree.insert((*path).to_owned(), (*text).to_owned());
            }
            let tree_id = self.id();
            self.trees.insert(tree_id.clone(), tree);
            let commit = self.id();
            let parent = (!self.head.is_empty()).then(|| self.head.clone());
            self.commits
                .insert(commit.clone(), (tree_id, parent, message.to_owned()));
            self.head = commit.clone();
            commit
        }

        pub fn files(&self, commit: &str) -> Files {
            self.commits
                .get(commit)
                .and_then(|(tree, _, _)| self.trees.get(tree))
                .cloned()
                .unwrap_or_default()
        }

        pub fn message(&self, commit: &str) -> String {
            self.commits
                .get(commit)
                .map(|c| c.2.clone())
                .unwrap_or_default()
        }

        pub fn commit_count(&self) -> usize {
            self.commits.len()
        }
    }

    pub type Shared = Arc<Mutex<Repo>>;

    fn resource(name: &str, singular: &str, kind: &str, path: &str, singleton: bool) -> Value {
        let mut verbs = vec![
            "get", "describe", "explain", "edit", "apply", "diff", "patch",
        ];
        if !singleton {
            verbs.push("delete");
        }
        json!({
            "name": name, "singular": singular, "short_names": [], "kind": kind,
            "singleton": singleton, "verbs": verbs,
            "source": {"path": path},
            "columns": [],
        })
    }

    /// The page type the hub's content declares, as the server lists it.
    fn project_kind() -> Value {
        json!({"kind": "PageKind", "metadata": {"name": "project"}, "status": {}, "spec": {
            "names": {"kind": "Project", "singular": "project", "plural": "projects"},
            "fields": {
                "tagline": {"type": "string", "required": true},
                "status": {"type": "enum", "values": ["active", "wip", "archived"], "required": true},
                "tags": {"type": "list"},
            },
        }})
    }

    pub fn serve(repo: Shared) -> String {
        let app = Router::new()
            .route(
                "/api/v1/api-resources",
                get(|| async {
                    Json(json!({"kind": "APIResourceList", "items": [
                        resource("site", "site", "Site", "site.yaml", true),
                        resource("pages", "page", "Page", "pages/{name}.yaml", false),
                        resource("projects", "project", "Project", "projects/{name}.yaml", false),
                    ]}))
                }),
            )
            .route(
                "/api/v1/pagekinds",
                get(|| async { Json(json!({"kind": "PageKindList", "items": [project_kind()]})) }),
            )
            .route(
                "/api/v1/collections",
                get(|| async {
                    Json(json!({"kind": "CollectionList", "items": [
                        {"kind": "Collection", "metadata": {"name": "projects"}, "status": {},
                         "spec": {"kind": "Project", "enabled": true}},
                    ]}))
                }),
            )
            .route(
                "/healthz",
                get(|| async { Json(json!({"status": "ok", "version": "dev"})) }),
            )
            .route("/api/v1/schema", get(|| async { Json(json!({})) }))
            .route("/api/v1/sync", get(status).post(sync))
            .route("/repos/o/r/git/ref/heads/main", get(head))
            .route("/repos/o/r/contents/{*path}", get(contents))
            .route("/repos/o/r/git/commits/{sha}", get(commit))
            .route("/repos/o/r/git/trees", post(tree))
            .route("/repos/o/r/git/commits", post(new_commit))
            .route("/repos/o/r/git/refs/heads/main", patch(move_ref))
            .with_state(repo);
        spawn(app)
    }

    fn status_body(repo: &Repo) -> Value {
        json!({
            "kind": "SyncStatus", "source": "github.com/o/r@main",
            "repository": "o/r", "branch": "main", "revision": repo.head,
            "synced_at": "t", "last_attempt_at": "t", "last_error": null,
        })
    }

    async fn status(State(repo): State<Shared>) -> Json<Value> {
        Json(status_body(&repo.lock().unwrap()))
    }

    async fn sync(State(repo): State<Shared>, body: Option<Json<Value>>) -> Json<Value> {
        let mut repo = repo.lock().unwrap();
        let revision = body.and_then(|Json(b)| b["revision"].as_str().map(str::to_owned));
        repo.synced.push(revision);
        let mut answer = status_body(&repo);
        answer["report"] =
            json!({"kinds": {"Project": {"upserted": 1, "skipped": 0, "deleted": 0}}});
        Json(answer)
    }

    async fn head(State(repo): State<Shared>) -> Json<Value> {
        Json(json!({"object": {"sha": repo.lock().unwrap().head}}))
    }

    async fn contents(
        State(repo): State<Shared>,
        Path(path): Path<String>,
        Query(query): Query<HashMap<String, String>>,
    ) -> Response {
        let repo = repo.lock().unwrap();
        match repo.files(&query["ref"]).get(&path) {
            Some(text) => text.clone().into_response(),
            None => (StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"}))).into_response(),
        }
    }

    async fn commit(State(repo): State<Shared>, Path(sha): Path<String>) -> Response {
        let repo = repo.lock().unwrap();
        match repo.commits.get(&sha) {
            Some((tree, _, _)) => Json(json!({"sha": sha, "tree": {"sha": tree}})).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    fn authorized(headers: &HeaderMap) -> bool {
        headers
            .get("authorization")
            .is_some_and(|v| v == "Bearer t0ken")
    }

    async fn tree(
        State(repo): State<Shared>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Response {
        if !authorized(&headers) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"message": "Bad credentials"})),
            )
                .into_response();
        }
        let mut repo = repo.lock().unwrap();
        let mut files = repo
            .trees
            .get(body["base_tree"].as_str().unwrap())
            .cloned()
            .unwrap_or_default();
        for entry in body["tree"].as_array().unwrap() {
            let path = entry["path"].as_str().unwrap().to_owned();
            match entry["content"].as_str() {
                Some(text) => {
                    files.insert(path, text.to_owned());
                }
                None => {
                    files.remove(&path);
                }
            }
        }
        let id = repo.id();
        repo.trees.insert(id.clone(), files);
        (StatusCode::CREATED, Json(json!({"sha": id}))).into_response()
    }

    async fn new_commit(
        State(repo): State<Shared>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Response {
        if !authorized(&headers) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let mut repo = repo.lock().unwrap();
        let id = repo.id();
        let parent = body["parents"][0].as_str().map(str::to_owned);
        let tree = body["tree"].as_str().unwrap().to_owned();
        let message = body["message"].as_str().unwrap().to_owned();
        repo.commits.insert(id.clone(), (tree, parent, message));
        (StatusCode::CREATED, Json(json!({"sha": id}))).into_response()
    }

    async fn move_ref(
        State(repo): State<Shared>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Response {
        if !authorized(&headers) || body["force"] != false {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let mut repo = repo.lock().unwrap();
        let sha = body["sha"].as_str().unwrap().to_owned();
        let parent = repo.commits.get(&sha).and_then(|c| c.1.clone());
        if parent.as_deref() != Some(repo.head.as_str()) {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"message": "Update is not a fast forward"})),
            )
                .into_response();
        }
        repo.head = sha.clone();
        Json(json!({"object": {"sha": sha}})).into_response()
    }
}
