//! Page views without tracking: counts per UTC day, route and referring
//! host, and distinct visitors per day.
//!
//! A visitor is `sha256(salt, address, user agent)`, where the salt is 32
//! random bytes that live only as long as their day. The server counts into
//! a [`Batch`] in memory and flushes it now and then; the first flush after
//! a day ends closes it: its visitors are counted, and its hashes and salt
//! deleted. Every write is additive or idempotent, so the two servers of a
//! deploy can flush into one database.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use sha2::{Digest, Sha256};

use crate::api::{self, Since};
use crate::{Db, Error};

/// The route every request answered 404 is counted under, so that scanners
/// cannot fill the table with paths.
pub const NOT_FOUND: &str = "404";

/// How long after midnight a day stays open: a server flushes what it
/// counted before midnight within a minute, so none of it reaches a closed
/// day.
pub const GRACE: TimeDelta = TimeDelta::minutes(5);

/// One visitor, as a day's salt disguises it.
pub type VisitorHash = [u8; 32];

/// What a server counted since its last flush.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    /// Views per (day, route).
    pub views: BTreeMap<(String, String), u64>,
    /// Views per (day, referring host).
    pub referrers: BTreeMap<(String, String), u64>,
    /// Bot requests per day.
    pub bots: BTreeMap<String, u64>,
    /// Who viewed each (day, route).
    pub visitors: BTreeSet<(String, String, VisitorHash)>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
            && self.referrers.is_empty()
            && self.bots.is_empty()
            && self.visitors.is_empty()
    }

    /// Adds `other` back, as after a flush that failed.
    pub fn merge(&mut self, other: Batch) {
        for (key, views) in other.views {
            *self.views.entry(key).or_default() += views;
        }
        for (key, views) in other.referrers {
            *self.referrers.entry(key).or_default() += views;
        }
        for (day, bots) in other.bots {
            *self.bots.entry(day).or_default() += bots;
        }
        self.visitors.extend(other.visitors);
    }
}

/// A UTC day as the tables key it, `YYYY-MM-DD`.
pub fn day(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d").to_string()
}

/// Today, UTC.
pub fn today() -> String {
    day(Utc::now())
}

/// The hash a visitor is known by for one day.
pub fn visitor(salt: &[u8], address: &str, user_agent: &str) -> VisitorHash {
    let mut hash = Sha256::new();
    hash.update(salt);
    hash.update(address.as_bytes());
    // Neither an address nor a user agent contains NUL, so the pair cannot
    // be read two ways.
    hash.update([0]);
    hash.update(user_agent.as_bytes());
    hash.finalize().into()
}

#[derive(sqlx::FromRow)]
struct DayRow {
    day: String,
    views: i64,
    visitors: i64,
    bots: i64,
}

impl Db {
    /// The salt of `day`, created by whichever server asks first.
    pub async fn salt(&self, day: &str) -> Result<Vec<u8>, Error> {
        // SQLite's randomness is seeded from the operating system.
        sqlx::query("INSERT OR IGNORE INTO metrics_salt (day, salt) VALUES (?1, randomblob(32))")
            .bind(day)
            .execute(self.pool())
            .await?;
        Ok(
            sqlx::query_scalar("SELECT salt FROM metrics_salt WHERE day = ?1")
                .bind(day)
                .fetch_one(self.pool())
                .await?,
        )
    }

    /// Writes a batch and closes the days that ended; answers the days it
    /// closed.
    pub async fn flush_metrics(&self, batch: &Batch) -> Result<Vec<String>, Error> {
        self.flush_metrics_at(batch, Utc::now()).await
    }

    async fn flush_metrics_at(
        &self,
        batch: &Batch,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>, Error> {
        let mut tx = self.pool().begin().await?;
        // Writes only, from the first statement on: the transaction takes the
        // write lock at once, and never has to upgrade a read.
        let mut days: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
        for ((day, route), views) in &batch.views {
            sqlx::query(
                "INSERT INTO page_views (day, route, views) VALUES (?1, ?2, ?3)
                 ON CONFLICT (day, route) DO UPDATE SET views = views + excluded.views",
            )
            .bind(day)
            .bind(route)
            .bind(count(*views)?)
            .execute(&mut *tx)
            .await?;
            let entry = days.entry(day).or_default();
            if route != NOT_FOUND {
                entry.0 += views;
            }
        }
        for (day, bots) in &batch.bots {
            days.entry(day).or_default().1 += bots;
        }
        for (day, (views, bots)) in days {
            sqlx::query(
                "INSERT INTO daily (day, views, bots) VALUES (?1, ?2, ?3)
                 ON CONFLICT (day) DO UPDATE
                 SET views = views + excluded.views, bots = bots + excluded.bots",
            )
            .bind(day)
            .bind(count(views)?)
            .bind(count(bots)?)
            .execute(&mut *tx)
            .await?;
        }
        for ((day, host), views) in &batch.referrers {
            sqlx::query(
                "INSERT INTO referrers (day, host, views) VALUES (?1, ?2, ?3)
                 ON CONFLICT (day, host) DO UPDATE SET views = views + excluded.views",
            )
            .bind(day)
            .bind(host)
            .bind(count(*views)?)
            .execute(&mut *tx)
            .await?;
        }
        // A day already closed has counted its visitors: one seen again after
        // that could not be told from the ones counted, so it is dropped.
        for (day, route, hash) in &batch.visitors {
            sqlx::query(
                "INSERT OR IGNORE INTO visitor_hashes (day, route, hash)
                 SELECT ?1, ?2, ?3
                 WHERE NOT EXISTS (SELECT 1 FROM daily WHERE day = ?1 AND closed = 1)",
            )
            .bind(day)
            .bind(route)
            .bind(&hash[..])
            .execute(&mut *tx)
            .await?;
        }

        let open_until = day(now - GRACE);
        sqlx::query(
            "UPDATE page_views SET visitors = (
                 SELECT count(*) FROM visitor_hashes AS h
                 WHERE h.day = page_views.day AND h.route = page_views.route
             )
             WHERE day IN (SELECT day FROM daily WHERE closed = 0 AND day < ?1)",
        )
        .bind(&open_until)
        .execute(&mut *tx)
        .await?;
        let mut closed: Vec<String> = sqlx::query_scalar(
            "UPDATE daily SET closed = 1, visitors = (
                 SELECT count(DISTINCT hash) FROM visitor_hashes AS h
                 WHERE h.day = daily.day AND h.route != ?2
             )
             WHERE closed = 0 AND day < ?1
             RETURNING day",
        )
        .bind(&open_until)
        .bind(NOT_FOUND)
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM visitor_hashes WHERE day < ?1")
            .bind(&open_until)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM metrics_salt WHERE day < ?1")
            .bind(&open_until)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        // RETURNING follows no order.
        closed.sort();
        Ok(closed)
    }

    /// Visits per day over `since`, every day of it listed.
    pub async fn daily_metrics(&self, since: Since) -> Result<api::DailyMetrics, Error> {
        self.daily_metrics_at(since, Utc::now()).await
    }

    async fn daily_metrics_at(
        &self,
        since: Since,
        now: DateTime<Utc>,
    ) -> Result<api::DailyMetrics, Error> {
        let mut tx = self.pool().begin().await?;
        let period = period(&mut tx, since, now).await?;
        // An open day's visitors are the hashes it holds so far.
        let rows: Vec<DayRow> = sqlx::query_as(
            "SELECT day, views, bots, CASE WHEN closed = 1 THEN visitors ELSE (
                 SELECT count(DISTINCT hash) FROM visitor_hashes AS h
                 WHERE h.day = daily.day AND h.route != ?3
             ) END AS visitors
             FROM daily WHERE day BETWEEN ?1 AND ?2 ORDER BY day",
        )
        .bind(&period.from)
        .bind(&period.to)
        .bind(NOT_FOUND)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let mut rows = rows.into_iter().peekable();
        let mut items = Vec::new();
        let mut total = api::Visits::default();
        for day in days(&period.from, &period.to)? {
            let visits = match rows.next_if(|row| row.day == day) {
                Some(row) => api::Visits {
                    views: unsigned(row.views)?,
                    visitors: unsigned(row.visitors)?,
                    bots: unsigned(row.bots)?,
                },
                None => api::Visits::default(),
            };
            total.views += visits.views;
            total.visitors += visits.visitors;
            total.bots += visits.bots;
            items.push(api::DayVisits { day, visits });
        }
        Ok(api::DailyMetrics {
            kind: "DailyMetrics".to_owned(),
            period,
            total,
            items,
        })
    }

    /// Views and visitors per route over `since`, most viewed first.
    pub async fn page_metrics(&self, since: Since) -> Result<api::PageMetrics, Error> {
        self.page_metrics_at(since, Utc::now()).await
    }

    async fn page_metrics_at(
        &self,
        since: Since,
        now: DateTime<Utc>,
    ) -> Result<api::PageMetrics, Error> {
        let mut tx = self.pool().begin().await?;
        let period = period(&mut tx, since, now).await?;
        let rows: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT p.route, sum(p.views), sum(CASE WHEN d.closed = 1 THEN p.visitors ELSE (
                 SELECT count(*) FROM visitor_hashes AS h
                 WHERE h.day = p.day AND h.route = p.route
             ) END)
             FROM page_views AS p LEFT JOIN daily AS d ON d.day = p.day
             WHERE p.day BETWEEN ?1 AND ?2
             GROUP BY p.route ORDER BY 2 DESC, p.route",
        )
        .bind(&period.from)
        .bind(&period.to)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(api::PageMetrics {
            kind: "PageMetrics".to_owned(),
            period,
            items: rows
                .into_iter()
                .map(|(route, views, visitors)| {
                    Ok(api::PageVisits {
                        route,
                        views: unsigned(views)?,
                        visitors: unsigned(visitors)?,
                    })
                })
                .collect::<Result<_, Error>>()?,
        })
    }

    /// Views per referring host over `since`, most first.
    pub async fn referrer_metrics(&self, since: Since) -> Result<api::ReferrerMetrics, Error> {
        self.referrer_metrics_at(since, Utc::now()).await
    }

    async fn referrer_metrics_at(
        &self,
        since: Since,
        now: DateTime<Utc>,
    ) -> Result<api::ReferrerMetrics, Error> {
        let mut tx = self.pool().begin().await?;
        let period = period(&mut tx, since, now).await?;
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT host, sum(views) FROM referrers WHERE day BETWEEN ?1 AND ?2
             GROUP BY host ORDER BY 2 DESC, host",
        )
        .bind(&period.from)
        .bind(&period.to)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(api::ReferrerMetrics {
            kind: "ReferrerMetrics".to_owned(),
            period,
            items: rows
                .into_iter()
                .map(|(host, views)| {
                    Ok(api::ReferrerVisits {
                        host,
                        views: unsigned(views)?,
                    })
                })
                .collect::<Result<_, Error>>()?,
        })
    }
}

/// The days `since` covers, up to today; `all` starts at the first day
/// kept, or today when there is none.
async fn period(
    tx: &mut sqlx::SqliteConnection,
    since: Since,
    now: DateTime<Utc>,
) -> Result<api::Period, Error> {
    let to = day(now);
    let from = match since {
        Since::Days(days) => day(now - TimeDelta::days(i64::from(days) - 1)),
        Since::All => {
            let first: Option<String> = sqlx::query_scalar(
                "SELECT min(day) FROM (
                     SELECT min(day) AS day FROM daily
                     UNION ALL SELECT min(day) FROM page_views
                     UNION ALL SELECT min(day) FROM referrers
                 )",
            )
            .fetch_one(&mut *tx)
            .await?;
            first
                .filter(|first| *first < to)
                .unwrap_or_else(|| to.clone())
        }
    };
    Ok(api::Period {
        since: since.to_string(),
        from,
        to,
    })
}

/// Every day from `from` to `to`, both included.
fn days(from: &str, to: &str) -> Result<Vec<String>, Error> {
    let parse = |day: &str| {
        NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .map_err(|_| Error::Invariant("a metrics day is not YYYY-MM-DD"))
    };
    let (from, to) = (parse(from)?, parse(to)?);
    Ok(from
        .iter_days()
        .take_while(|day| *day <= to)
        .map(|day| day.format("%Y-%m-%d").to_string())
        .collect())
}

fn count(value: u64) -> Result<i64, Error> {
    i64::try_from(value).map_err(|_| Error::Invariant("a metrics count is out of range"))
}

fn unsigned(value: i64) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| Error::Invariant("a metrics count is negative"))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn open() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        (dir, db)
    }

    fn at(when: &str) -> DateTime<Utc> {
        when.parse().unwrap()
    }

    /// `views` of each route by one visitor, on `day`.
    fn batch(day: &str, visitor: u8, routes: &[(&str, u64)]) -> Batch {
        let mut batch = Batch::default();
        for (route, views) in routes {
            batch
                .views
                .insert((day.to_owned(), (*route).to_owned()), *views);
            batch
                .visitors
                .insert((day.to_owned(), (*route).to_owned(), [visitor; 32]));
        }
        batch
    }

    /// What is left of the open days: (hashes, salts).
    async fn open_rows(db: &Db) -> (i64, i64) {
        sqlx::query_as(
            "SELECT (SELECT count(*) FROM visitor_hashes), (SELECT count(*) FROM metrics_salt)",
        )
        .fetch_one(db.pool())
        .await
        .unwrap()
    }

    #[test]
    fn a_visitor_hash_depends_on_every_part() {
        let one = visitor(b"salt", "10.0.0.1", "firefox");
        assert_eq!(one, visitor(b"salt", "10.0.0.1", "firefox"));
        assert_ne!(one, visitor(b"pepper", "10.0.0.1", "firefox"));
        assert_ne!(one, visitor(b"salt", "10.0.0.2", "firefox"));
        assert_ne!(one, visitor(b"salt", "10.0.0.1", "chrome"));
        assert_ne!(
            visitor(b"salt", "10.0.0.1", "1firefox"),
            visitor(b"salt", "10.0.0.11", "firefox")
        );
    }

    #[test]
    fn batches_merge_by_adding() {
        let mut one = batch("2026-09-01", 1, &[("/", 2)]);
        one.bots.insert("2026-09-01".to_owned(), 1);
        let mut two = batch("2026-09-01", 2, &[("/", 3), ("/help", 1)]);
        two.bots.insert("2026-09-01".to_owned(), 4);
        one.merge(two);
        assert_eq!(one.views[&("2026-09-01".to_owned(), "/".to_owned())], 5);
        assert_eq!(one.bots["2026-09-01"], 5);
        assert_eq!(one.visitors.len(), 3);
        assert!(!one.is_empty());
        assert!(Batch::default().is_empty());
    }

    #[tokio::test]
    async fn every_server_gets_the_same_salt_for_a_day() {
        let (dir, db) = open().await;
        let other = Db::open(dir.path().join("hldr.db")).await.unwrap();
        let salt = db.salt("2026-09-01").await.unwrap();
        assert_eq!(salt.len(), 32);
        assert_eq!(other.salt("2026-09-01").await.unwrap(), salt);
        assert_ne!(db.salt("2026-09-02").await.unwrap(), salt);
    }

    #[tokio::test]
    async fn flushes_add_up_and_today_counts_live() {
        let (_dir, db) = open().await;
        let noon = at("2026-09-01T12:00:00Z");
        let mut first = batch("2026-09-01", 1, &[("/", 2), ("/help", 1)]);
        first.referrers.insert(
            ("2026-09-01".to_owned(), "news.ycombinator.com".to_owned()),
            2,
        );
        first.bots.insert("2026-09-01".to_owned(), 3);
        assert!(db.flush_metrics_at(&first, noon).await.unwrap().is_empty());
        let mut second = batch("2026-09-01", 2, &[("/", 1), (NOT_FOUND, 4)]);
        second.merge(batch("2026-09-01", 1, &[("/", 1)]));
        db.flush_metrics_at(&second, noon).await.unwrap();

        let daily = db.daily_metrics_at(Since::Days(1), noon).await.unwrap();
        assert_eq!(
            daily.items,
            [api::DayVisits {
                day: "2026-09-01".to_owned(),
                visits: api::Visits {
                    views: 5,
                    visitors: 2,
                    bots: 3
                },
            }],
            "404s are views of their route only"
        );
        let pages = db.page_metrics_at(Since::Days(1), noon).await.unwrap();
        let pages: Vec<(&str, u64, u64)> = pages
            .items
            .iter()
            .map(|page| (page.route.as_str(), page.views, page.visitors))
            .collect();
        assert_eq!(pages, [("/", 4, 2), (NOT_FOUND, 4, 1), ("/help", 1, 1)]);
        let referrers = db.referrer_metrics_at(Since::Days(1), noon).await.unwrap();
        assert_eq!(referrers.items[0].host, "news.ycombinator.com");
        assert_eq!(referrers.items[0].views, 2);
    }

    #[tokio::test]
    async fn a_day_closes_once_after_the_grace_and_keeps_only_counts() {
        let (dir, db) = open().await;
        let other = Db::open(dir.path().join("hldr.db")).await.unwrap();
        db.salt("2026-09-01").await.unwrap();
        let day_one = batch("2026-09-01", 1, &[("/", 1), ("/help", 1)]);
        db.flush_metrics_at(&day_one, at("2026-09-01T23:59:00Z"))
            .await
            .unwrap();
        other
            .flush_metrics_at(
                &batch("2026-09-01", 2, &[("/", 1)]),
                at("2026-09-02T00:01:00Z"),
            )
            .await
            .unwrap();
        assert_eq!(open_rows(&db).await, (3, 1), "still open");

        let after = at("2026-09-02T00:05:00Z");
        let closed = db.flush_metrics_at(&Batch::default(), after).await.unwrap();
        assert_eq!(closed, ["2026-09-01"]);
        assert!(
            other
                .flush_metrics_at(&Batch::default(), after)
                .await
                .unwrap()
                .is_empty(),
            "closing twice does nothing"
        );
        assert_eq!(open_rows(&db).await, (0, 0));

        // A late flush of the closed day adds its views but not its visitor.
        other
            .flush_metrics_at(&batch("2026-09-01", 3, &[("/", 1)]), after)
            .await
            .unwrap();
        assert_eq!(open_rows(&db).await, (0, 0));
        let daily = db.daily_metrics_at(Since::Days(2), after).await.unwrap();
        assert_eq!(
            daily.items[0].visits,
            api::Visits {
                views: 4,
                visitors: 2,
                bots: 0
            }
        );
        assert_eq!(daily.items[1].visits, api::Visits::default());
        let pages = db.page_metrics_at(Since::Days(2), after).await.unwrap();
        assert_eq!(pages.items[0].route, "/");
        assert_eq!((pages.items[0].views, pages.items[0].visitors), (3, 2));
    }

    #[tokio::test]
    async fn a_period_lists_every_day_and_all_starts_at_the_first() {
        let (_dir, db) = open().await;
        db.flush_metrics_at(
            &batch("2026-08-30", 1, &[("/", 1)]),
            at("2026-08-30T10:00:00Z"),
        )
        .await
        .unwrap();
        let now = at("2026-09-02T10:00:00Z");
        db.flush_metrics_at(&Batch::default(), now).await.unwrap();
        let week = db.daily_metrics_at(Since::Days(7), now).await.unwrap();
        assert_eq!(
            week.period,
            api::Period {
                since: "7d".to_owned(),
                from: "2026-08-27".to_owned(),
                to: "2026-09-02".to_owned()
            }
        );
        assert_eq!(week.items.len(), 7);
        assert_eq!(week.total.views, 1);
        assert_eq!(week.total.visitors, 1, "a closed day keeps its visitors");

        let all = db.daily_metrics_at(Since::All, now).await.unwrap();
        assert_eq!(all.period.from, "2026-08-30");
        assert_eq!(all.items.len(), 4);
        let today = db.daily_metrics_at(Since::Days(1), now).await.unwrap();
        assert_eq!(today.items.len(), 1);
        assert_eq!(today.total, api::Visits::default());

        let (_empty_dir, empty) = open().await;
        let nothing = empty.daily_metrics_at(Since::All, now).await.unwrap();
        assert_eq!(nothing.period.from, "2026-09-02");
        assert!(
            empty
                .page_metrics_at(Since::All, now)
                .await
                .unwrap()
                .items
                .is_empty()
        );
    }
}
