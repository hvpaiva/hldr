//! Closed metrics days as the metrics repository keeps them: one JSON file
//! per day, counts only. No hash, salt, address or user agent ever reaches
//! a file, since a closed day holds none.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::store::now_rfc3339;
use crate::{Db, Error};

/// One closed day, at `days/YYYY/MM/YYYY-MM-DD.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayRecord {
    pub day: String,
    /// Pages served, 404s aside.
    pub views: u64,
    /// Distinct visitors of the day.
    pub visitors: u64,
    pub bots: u64,
    /// Views and visitors per route, `404` included.
    pub pages: BTreeMap<String, PageCounts>,
    /// Views per referring host.
    pub referrers: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageCounts {
    pub views: u64,
    pub visitors: u64,
}

impl DayRecord {
    /// Where the day lives in the repository.
    pub fn path(&self) -> Result<String, Error> {
        path(&self.day)
    }

    /// The file's bytes: pretty JSON, keys in a fixed order, so the same day
    /// is always the same file.
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// A file of the repository, which must hold the day its path names.
    pub fn from_file(path: &str, bytes: &[u8]) -> Result<Self, Error> {
        let record: Self = serde_json::from_slice(bytes)
            .map_err(|err| Error::file(path, format!("not a metrics day: {err}")))?;
        if record.path().ok().as_deref() != Some(path) {
            return Err(Error::file(path, "holds another day than its name"));
        }
        Ok(record)
    }
}

/// `days/YYYY/MM/YYYY-MM-DD.json` for a day written `YYYY-MM-DD`.
pub fn path(day: &str) -> Result<String, Error> {
    chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .ok()
        .filter(|date| date.format("%Y-%m-%d").to_string() == day)
        .ok_or(Error::Invariant("a metrics day is not YYYY-MM-DD"))?;
    Ok(format!("days/{}/{}/{day}.json", &day[..4], &day[5..7]))
}

fn count(value: i64) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| Error::Invariant("a metrics count is negative"))
}

fn signed(value: u64) -> Result<i64, Error> {
    i64::try_from(value).map_err(|_| Error::Invariant("a metrics count is out of range"))
}

impl Db {
    /// Closed days the repository does not hold yet, oldest first.
    pub async fn pending_backups(&self) -> Result<Vec<String>, Error> {
        Ok(sqlx::query_scalar(
            "SELECT day FROM daily
             WHERE closed = 1 AND day NOT IN (SELECT day FROM metrics_backup)
             ORDER BY day",
        )
        .fetch_all(self.pool())
        .await?)
    }

    /// Whether any day was ever backed up or restored: a database that never
    /// was is restored from the repository first.
    pub async fn has_backups(&self) -> Result<bool, Error> {
        Ok(
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM metrics_backup)")
                .fetch_one(self.pool())
                .await?,
        )
    }

    /// The latest day the repository holds.
    pub async fn last_backup(&self) -> Result<Option<String>, Error> {
        Ok(sqlx::query_scalar("SELECT max(day) FROM metrics_backup")
            .fetch_one(self.pool())
            .await?)
    }

    /// A closed day as its file holds it; none when the day is open or
    /// unknown.
    pub async fn day_record(&self, day: &str) -> Result<Option<DayRecord>, Error> {
        let mut tx = self.pool().begin().await?;
        let Some((views, visitors, bots)): Option<(i64, i64, i64)> =
            sqlx::query_as("SELECT views, visitors, bots FROM daily WHERE day = ?1 AND closed = 1")
                .bind(day)
                .fetch_optional(&mut *tx)
                .await?
        else {
            return Ok(None);
        };
        let pages: Vec<(String, i64, i64)> =
            sqlx::query_as("SELECT route, views, visitors FROM page_views WHERE day = ?1")
                .bind(day)
                .fetch_all(&mut *tx)
                .await?;
        let referrers: Vec<(String, i64)> =
            sqlx::query_as("SELECT host, views FROM referrers WHERE day = ?1")
                .bind(day)
                .fetch_all(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(Some(DayRecord {
            day: day.to_owned(),
            views: count(views)?,
            visitors: count(visitors)?,
            bots: count(bots)?,
            pages: pages
                .into_iter()
                .map(|(route, views, visitors)| {
                    Ok((
                        route,
                        PageCounts {
                            views: count(views)?,
                            visitors: count(visitors)?,
                        },
                    ))
                })
                .collect::<Result<_, Error>>()?,
            referrers: referrers
                .into_iter()
                .map(|(host, views)| Ok((host, count(views)?)))
                .collect::<Result<_, Error>>()?,
        }))
    }

    /// Notes that the repository holds `day`, as the blob `sha`.
    pub async fn record_backup(&self, day: &str, sha: &str) -> Result<(), Error> {
        sqlx::query(
            "INSERT OR REPLACE INTO metrics_backup (day, uploaded_at, sha) VALUES (?1, ?2, ?3)",
        )
        .bind(day)
        .bind(now_rfc3339())
        .bind(sha)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Loads days from the repository, in one transaction, and answers how
    /// many were new. A day the database already has keeps its own counts.
    pub async fn restore_days(&self, records: &[DayRecord]) -> Result<usize, Error> {
        let now = now_rfc3339();
        let mut tx = self.pool().begin().await?;
        let mut restored = 0;
        for record in records {
            let inserted = sqlx::query(
                "INSERT INTO daily (day, views, visitors, bots, closed) VALUES (?1, ?2, ?3, ?4, 1)
                 ON CONFLICT (day) DO NOTHING",
            )
            .bind(&record.day)
            .bind(signed(record.views)?)
            .bind(signed(record.visitors)?)
            .bind(signed(record.bots)?)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if inserted == 0 {
                continue;
            }
            for (route, page) in &record.pages {
                sqlx::query(
                    "INSERT INTO page_views (day, route, views, visitors) VALUES (?1, ?2, ?3, ?4)",
                )
                .bind(&record.day)
                .bind(route)
                .bind(signed(page.views)?)
                .bind(signed(page.visitors)?)
                .execute(&mut *tx)
                .await?;
            }
            for (host, views) in &record.referrers {
                sqlx::query("INSERT INTO referrers (day, host, views) VALUES (?1, ?2, ?3)")
                    .bind(&record.day)
                    .bind(host)
                    .bind(signed(*views)?)
                    .execute(&mut *tx)
                    .await?;
            }
            sqlx::query("INSERT OR IGNORE INTO metrics_backup (day, uploaded_at) VALUES (?1, ?2)")
                .bind(&record.day)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            restored += 1;
        }
        tx.commit().await?;
        Ok(restored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Since;
    use crate::metrics::Batch;

    async fn open() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        (dir, db)
    }

    fn record() -> DayRecord {
        DayRecord {
            day: "2026-09-01".to_owned(),
            views: 3,
            visitors: 2,
            bots: 7,
            pages: BTreeMap::from([
                (
                    "/".to_owned(),
                    PageCounts {
                        views: 3,
                        visitors: 2,
                    },
                ),
                (
                    "404".to_owned(),
                    PageCounts {
                        views: 1,
                        visitors: 1,
                    },
                ),
            ]),
            referrers: BTreeMap::from([("lobste.rs".to_owned(), 2)]),
        }
    }

    #[test]
    fn a_day_is_one_stable_file() {
        let record = record();
        assert_eq!(record.path().unwrap(), "days/2026/09/2026-09-01.json");
        let bytes = record.to_bytes().unwrap();
        assert_eq!(
            String::from_utf8(bytes.clone()).unwrap(),
            r#"{
  "day": "2026-09-01",
  "views": 3,
  "visitors": 2,
  "bots": 7,
  "pages": {
    "/": {
      "views": 3,
      "visitors": 2
    },
    "404": {
      "views": 1,
      "visitors": 1
    }
  },
  "referrers": {
    "lobste.rs": 2
  }
}
"#
        );
        let path = "days/2026/09/2026-09-01.json";
        assert_eq!(DayRecord::from_file(path, &bytes).unwrap(), record);
        assert!(DayRecord::from_file("days/2026/09/2026-09-02.json", &bytes).is_err());
        assert!(DayRecord::from_file(path, b"{}").is_err());
        for bad in ["2026-9-1", "2026-02-30", "../2026-09-01", ""] {
            assert!(super::path(bad).is_err(), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn a_closed_day_becomes_a_record_and_a_restore_brings_it_back() {
        let (_dir, db) = open().await;
        let mut batch = Batch::default();
        let day = "2026-09-01".to_owned();
        batch.views.insert((day.clone(), "/".to_owned()), 3);
        batch.views.insert((day.clone(), "404".to_owned()), 1);
        batch
            .visitors
            .insert((day.clone(), "/".to_owned(), [1; 32]));
        batch
            .visitors
            .insert((day.clone(), "/".to_owned(), [2; 32]));
        batch
            .visitors
            .insert((day.clone(), "404".to_owned(), [1; 32]));
        batch
            .referrers
            .insert((day.clone(), "lobste.rs".to_owned()), 2);
        batch.bots.insert(day.clone(), 7);
        let at = |when: &str| when.parse::<chrono::DateTime<chrono::Utc>>().unwrap();
        db.flush_metrics_at(&batch, at("2026-09-01T12:00:00Z"))
            .await
            .unwrap();
        assert!(
            db.day_record(&day).await.unwrap().is_none(),
            "an open day is not backed up"
        );
        assert!(db.pending_backups().await.unwrap().is_empty());
        db.flush_metrics_at(&Batch::default(), at("2026-09-02T01:00:00Z"))
            .await
            .unwrap();
        assert_eq!(db.pending_backups().await.unwrap(), [day.as_str()]);
        let saved = db.day_record(&day).await.unwrap().unwrap();
        assert_eq!(saved, record());

        assert!(!db.has_backups().await.unwrap());
        db.record_backup(&day, "abc").await.unwrap();
        assert!(db.pending_backups().await.unwrap().is_empty());
        assert_eq!(
            db.last_backup().await.unwrap().as_deref(),
            Some("2026-09-01")
        );

        let (_other_dir, fresh) = open().await;
        let mut later = record();
        later.day = "2026-09-02".to_owned();
        assert_eq!(
            fresh
                .restore_days(&[record(), later.clone()])
                .await
                .unwrap(),
            2
        );
        assert_eq!(fresh.restore_days(&[record()]).await.unwrap(), 0, "once");
        assert_eq!(fresh.day_record(&day).await.unwrap().unwrap(), record());
        assert!(fresh.pending_backups().await.unwrap().is_empty());
        assert!(fresh.has_backups().await.unwrap());
        let all = fresh.daily_metrics(Since::All).await.unwrap();
        assert_eq!(all.items[0].visits.visitors, 2);
        assert_eq!(all.items[1].day, "2026-09-02");
    }
}
