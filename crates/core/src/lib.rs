use serde::Serialize;

pub mod db;
pub mod index;
pub mod store;
pub mod types;

mod error;
mod markdown;

pub use db::Db;
pub use error::Error;
pub use index::SyncReport;
pub use store::{Profile, Project, ProjectSummary};

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

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
    pub revision: &'static str,
}

impl Health {
    pub fn ok() -> Self {
        Self {
            status: "ok",
            version: VERSION,
            revision: REVISION,
        }
    }
}
