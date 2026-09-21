use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[cfg(feature = "store")]
    #[error("sqlite: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[cfg(feature = "store")]
    #[error("migrate: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("yaml: {0}")]
    Yaml(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Frontmatter(&'static str),
    #[error("database invariant: {0}")]
    Invariant(&'static str),
    #[error("{path}: {message}", path = path.display())]
    File { path: PathBuf, message: String },
}

impl Error {
    pub fn file(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::File {
            path: path.into(),
            message: message.into(),
        }
    }
}
