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
    /// Several problems found in one pass, one per line.
    #[error("{}", join(.0))]
    Many(Vec<Error>),
}

impl Error {
    pub fn file(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::File {
            path: path.into(),
            message: message.into(),
        }
    }

    /// `Ok` when there is no problem, the problem itself when there is one,
    /// and [`Error::Many`] when there are more.
    pub fn collect(mut problems: Vec<Error>) -> Result<(), Error> {
        match problems.len() {
            0 => Ok(()),
            1 => Err(problems.remove(0)),
            _ => Err(Self::Many(problems)),
        }
    }

    /// The problems this error stands for, one per entry.
    pub fn problems(&self) -> Vec<&Error> {
        match self {
            Self::Many(problems) => problems.iter().flat_map(Error::problems).collect(),
            other => vec![other],
        }
    }
}

fn join(problems: &[Error]) -> String {
    problems
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}
