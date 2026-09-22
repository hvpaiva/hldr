use serde::Serialize;

pub mod api;
#[cfg(feature = "store")]
pub mod db;
pub mod github;
#[cfg(feature = "store")]
pub mod index;
pub mod manifest;
pub mod spec;
#[cfg(feature = "store")]
pub mod store;
pub mod template;
pub mod tree;
pub mod types;

mod error;
#[cfg(test)]
mod testing;

pub use api::SyncReport;
#[cfg(feature = "store")]
pub use db::Db;
pub use error::Error;
#[cfg(feature = "store")]
pub use store::{Collection, Nav, Page, PageKind, Profile, Site, SyncState, Theme};

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
