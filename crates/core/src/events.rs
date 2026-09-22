//! What happened to the server: written as it happens, kept 30 days.
//!
//! The same event again right after itself is not a new row: its count and
//! `last_at` grow, as kubectl aggregates events, so a sync failing on every
//! poll stays one line. Every write takes the next `seq`, so a watch that
//! asks for what came after the last `seq` it saw sees updates as well as
//! new events.

use chrono::{DateTime, TimeDelta, Utc};
use sqlx::FromRow;

use crate::api::{self, EventType};
use crate::store::rfc3339;
use crate::{Db, Error};

/// How long an event is kept after it last happened.
pub const RETENTION: TimeDelta = TimeDelta::days(30);

/// An event about to be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEvent {
    pub event_type: EventType,
    pub reason: &'static str,
    pub message: String,
    pub revision: Option<String>,
}

impl NewEvent {
    pub fn normal(reason: &'static str, message: impl Into<String>) -> Self {
        Self::new(EventType::Normal, reason, message)
    }

    pub fn warning(reason: &'static str, message: impl Into<String>) -> Self {
        Self::new(EventType::Warning, reason, message)
    }

    fn new(event_type: EventType, reason: &'static str, message: impl Into<String>) -> Self {
        Self {
            event_type,
            reason,
            message: message.into(),
            revision: None,
        }
    }

    /// The content revision served when it happened.
    pub fn at(mut self, revision: Option<&str>) -> Self {
        self.revision = revision.map(str::to_owned);
        self
    }
}

#[derive(FromRow)]
struct EventRow {
    id: i64,
    seq: i64,
    #[sqlx(rename = "type")]
    event_type: String,
    reason: String,
    message: String,
    revision: Option<String>,
    count: i64,
    first_at: String,
    last_at: String,
}

// Every column of `events`, then the rest of the query.
macro_rules! events {
    ($rest:literal) => {
        concat!(
            "SELECT id, seq, type, reason, message, revision, count, first_at, last_at FROM events",
            $rest
        )
    };
}

impl Db {
    /// Writes an event, or counts it again when it repeats the last one.
    pub async fn record_event(&self, event: &NewEvent) -> Result<(), Error> {
        self.record_event_at(event, Utc::now()).await
    }

    async fn record_event_at(&self, event: &NewEvent, now: DateTime<Utc>) -> Result<(), Error> {
        let at = rfc3339(now);
        let mut tx = self.pool().begin().await?;
        // A write first: the transaction holds the write lock from here on,
        // so another server on the same database cannot interleave.
        let seq: i64 =
            sqlx::query_scalar("UPDATE event_seq SET seq = seq + 1 WHERE id = 1 RETURNING seq")
                .fetch_one(&mut *tx)
                .await?;
        let last: Option<EventRow> = sqlx::query_as(events!(" ORDER BY seq DESC LIMIT 1"))
            .fetch_optional(&mut *tx)
            .await?;
        let repeat = last.filter(|last| {
            last.event_type == event.event_type.as_str()
                && last.reason == event.reason
                && last.message == event.message
                && last.revision == event.revision
        });
        match repeat {
            Some(last) => {
                sqlx::query(
                    "UPDATE events SET seq = ?1, count = count + 1, last_at = ?2 WHERE id = ?3",
                )
                .bind(seq)
                .bind(&at)
                .bind(last.id)
                .execute(&mut *tx)
                .await?;
            }
            None => {
                sqlx::query(
                    "INSERT INTO events (seq, type, reason, message, revision, first_at, last_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                )
                .bind(seq)
                .bind(event.event_type.as_str())
                .bind(event.reason)
                .bind(&event.message)
                .bind(&event.revision)
                .bind(&at)
                .execute(&mut *tx)
                .await?;
            }
        }
        sqlx::query("DELETE FROM events WHERE last_at < ?1")
            .bind(rfc3339(now - RETENTION))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Events written after `after`, or all of them, oldest first, with the
    /// stream position they were read at. One read transaction, so no
    /// event lands between the list and the position.
    pub async fn events(&self, after: Option<i64>) -> Result<api::EventList, Error> {
        let mut tx = self.pool().begin().await?;
        let rows: Vec<EventRow> = sqlx::query_as(events!(" WHERE seq > ?1 ORDER BY seq"))
            .bind(after.unwrap_or(0))
            .fetch_all(&mut *tx)
            .await?;
        let seq: i64 = sqlx::query_scalar("SELECT seq FROM event_seq WHERE id = 1")
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(api::EventList {
            kind: "EventList".to_owned(),
            seq,
            items: rows
                .into_iter()
                .map(event_from_row)
                .collect::<Result<_, _>>()?,
        })
    }

    /// One event, by the name the API gives it.
    pub async fn event(&self, id: i64) -> Result<Option<api::Event>, Error> {
        let row: Option<EventRow> = sqlx::query_as(events!(" WHERE id = ?1"))
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        row.map(event_from_row).transpose()
    }
}

fn event_from_row(row: EventRow) -> Result<api::Event, Error> {
    let event_type = match row.event_type.as_str() {
        "Normal" => EventType::Normal,
        "Warning" => EventType::Warning,
        _ => {
            return Err(Error::Invariant(
                "an event type is neither Normal nor Warning",
            ));
        }
    };
    Ok(api::Event {
        kind: "Event".to_owned(),
        metadata: api::EventMetadata {
            name: row.id.to_string(),
            seq: row.seq,
        },
        event_type,
        reason: row.reason,
        message: row.message,
        revision: row.revision,
        count: u32::try_from(row.count)
            .map_err(|_| Error::Invariant("an event count is out of range"))?,
        first_at: row.first_at,
        last_at: row.last_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        (dir, db)
    }

    fn day(n: u32) -> DateTime<Utc> {
        format!("2026-09-{n:02}T12:00:00Z").parse().unwrap()
    }

    #[tokio::test]
    async fn a_repeat_of_the_last_event_is_counted_not_added() {
        let (_dir, db) = db().await;
        let failed = NewEvent::warning("SyncFailed", "fetch: timeout").at(Some("abc"));
        db.record_event_at(&failed, day(1)).await.unwrap();
        db.record_event_at(&failed, day(2)).await.unwrap();
        let list = db.events(None).await.unwrap();
        assert_eq!(list.items.len(), 1);
        let event = &list.items[0];
        assert_eq!(event.count, 2);
        assert_eq!(event.first_at, "2026-09-01T12:00:00Z");
        assert_eq!(event.last_at, "2026-09-02T12:00:00Z");
        assert_eq!(event.metadata.seq, 2);
        assert_eq!(list.seq, 2);

        db.record_event_at(&NewEvent::normal("Synced", "abc → def"), day(3))
            .await
            .unwrap();
        db.record_event_at(&failed, day(4)).await.unwrap();
        let reasons: Vec<(String, u32)> = db
            .events(None)
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|event| (event.reason, event.count))
            .collect();
        assert_eq!(
            reasons,
            [
                ("SyncFailed".to_owned(), 2),
                ("Synced".to_owned(), 1),
                ("SyncFailed".to_owned(), 1)
            ],
            "only a repeat in a row is counted"
        );
    }

    #[tokio::test]
    async fn after_answers_updates_as_well_as_new_events() {
        let (_dir, db) = db().await;
        let started = NewEvent::normal("Started", "hldr-server dev started");
        db.record_event_at(&started, day(1)).await.unwrap();
        let cursor = db.events(None).await.unwrap().seq;
        assert!(db.events(Some(cursor)).await.unwrap().items.is_empty());

        db.record_event_at(&started, day(2)).await.unwrap();
        let update = db.events(Some(cursor)).await.unwrap();
        assert_eq!(update.items.len(), 1);
        assert_eq!(update.items[0].count, 2);
        assert_eq!(update.seq, cursor + 1);

        let id = update.items[0].metadata.name.parse().unwrap();
        assert_eq!(db.event(id).await.unwrap().unwrap().count, 2);
        assert!(db.event(id + 1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn events_older_than_the_retention_are_pruned_and_seq_never_repeats() {
        let (_dir, db) = db().await;
        db.record_event_at(&NewEvent::normal("Started", "one"), day(1))
            .await
            .unwrap();
        let late = day(1) + RETENTION + TimeDelta::seconds(1);
        db.record_event_at(&NewEvent::normal("Stopped", "two"), late)
            .await
            .unwrap();
        let list = db.events(None).await.unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].reason, "Stopped");
        assert_eq!(list.items[0].metadata.seq, 2);
    }
}
