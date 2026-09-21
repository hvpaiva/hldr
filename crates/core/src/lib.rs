use serde::Serialize;

pub mod api;
pub mod db;
pub mod index;
pub mod manifest;
pub mod store;
pub mod types;

mod error;
mod markdown;

pub use db::Db;
pub use error::Error;
pub use index::SyncReport;
pub use store::{Profile, Project, ProjectSummary, SiteConfig, SyncState};

/// Release version, stamped at build time through `HLDR_VERSION`.
///
/// The git tag is the source of truth; Cargo manifests carry no version.
/// Builds outside the release pipeline report `dev`.
pub const VERSION: &str = match option_env!("HLDR_VERSION") {
    Some(version) => version,
    None => "dev",
};

/// Git commit the binary was built from, stamped through `HLDR_REVISION`.
pub const REVISION: &str = match option_env!("HLDR_REVISION") {
    Some(revision) => revision,
    None => "unknown",
};

#[derive(Debug, Clone, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
    pub revision: &'static str,
    /// Content commit the database materializes, as `/readyz` reports it;
    /// absent from `/healthz`, for content read from a directory, and before
    /// the first sync.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_revision: Option<String>,
}

impl Health {
    pub fn ok(content_revision: Option<String>) -> Self {
        Self::new("ok", content_revision)
    }

    pub fn unready(content_revision: Option<String>) -> Self {
        Self::new("unready", content_revision)
    }

    fn new(status: &'static str, content_revision: Option<String>) -> Self {
        Self {
            status,
            version: VERSION,
            revision: REVISION,
            content_revision,
        }
    }
}
