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
}

pub fn resource(name: &str, singular: &str, kind: &str) -> Value {
    json!({
        "name": name, "singular": singular, "short_names": [], "kind": kind,
        "singleton": false, "verbs": ["get", "describe", "explain"],
        "source": {"path": format!("{name}/{{name}}.md"), "format": "markdown"},
        "columns": [{"name": "NAME", "json_path": ".metadata.name", "wide": false}],
    })
}

impl Stub {
    pub fn calls(&self) -> usize {
        self.discovery_calls.load(Ordering::SeqCst)
    }

    /// Serves on a random local port from a background thread and returns
    /// the base URL.
    pub fn serve(&self) -> String {
        let stub = self.clone();
        let app = Router::new()
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
}
