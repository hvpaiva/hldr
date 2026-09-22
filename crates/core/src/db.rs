use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use crate::Error;

#[derive(Clone, Debug)]
pub struct Db {
    pool: SqlitePool,
    path: PathBuf,
}

/// What the database takes on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    /// The main file.
    pub bytes: u64,
    /// The write-ahead log, not yet checkpointed into the main file.
    pub wal_bytes: u64,
}

impl Db {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_millis(5000));

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;

        sqlx::migrate!("../../migrations").run(&pool).await?;

        Ok(Self {
            pool,
            path: path.to_owned(),
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Sizes of the files on disk; a missing WAL, as after a checkpoint on
    /// close, counts as empty.
    pub fn size(&self) -> Result<Size, Error> {
        let bytes = std::fs::metadata(&self.path)?.len();
        let mut wal = self.path.clone().into_os_string();
        wal.push("-wal");
        let wal_bytes = match std::fs::metadata(&wal) {
            Ok(metadata) => metadata.len(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
            Err(err) => return Err(err.into()),
        };
        Ok(Size { bytes, wal_bytes })
    }
}
