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

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
}

impl Health {
    pub fn ok() -> Self {
        Self {
            status: "ok",
            version: VERSION,
        }
    }
}
