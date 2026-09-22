//! Reading the materialized content back, and the sync bookkeeping.

use std::cmp::Ordering;

use serde_json::Value;
use sqlx::FromRow;

use crate::manifest;
use crate::spec::{CollectionSpec, NavSpec, PageKindSpec, ProfileSpec, SiteSpec};
use crate::types::Palette;
use crate::{Db, Error};

#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    pub spec: SiteSpec,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub spec: ProfileSpec,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Nav {
    pub spec: NavSpec,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageKind {
    pub name: String,
    pub spec: PageKindSpec,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Collection {
    pub name: String,
    pub spec: CollectionSpec,
    pub updated_at: String,
}

/// A page of any type, without its markdown, which [`Db::content`] reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub page: manifest::Page,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub slug: String,
    pub title: String,
    pub dark: bool,
    /// Place in the picker; see [`crate::spec::ThemeSpec::order`].
    pub order: Option<i64>,
    pub hidden: bool,
    pub colors: Palette,
    pub updated_at: String,
}

/// Which content revision the database materializes.
#[derive(Debug, Clone, Default, PartialEq, Eq, FromRow)]
pub struct SyncState {
    pub revision: Option<String>,
    pub synced_at: Option<String>,
    pub last_attempt_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(FromRow)]
struct ThemeRow {
    slug: String,
    title: String,
    dark: bool,
    sort_order: Option<i64>,
    hidden: bool,
    colors: String,
    updated_at: String,
}

#[derive(FromRow)]
struct NamedRow {
    name: String,
    spec: String,
    updated_at: String,
}

#[derive(FromRow)]
struct PageRow {
    kind: String,
    name: String,
    collection: Option<String>,
    metadata: String,
    spec: String,
    created_at: String,
    updated_at: String,
}

// Every column of `themes` this release reads, then the rest of the query.
macro_rules! themes {
    ($rest:literal) => {
        concat!(
            "SELECT slug, title, dark, sort_order, hidden, colors, updated_at FROM themes",
            $rest
        )
    };
}

impl Db {
    pub async fn ping(&self) -> Result<(), Error> {
        sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(self.pool())
            .await?;
        Ok(())
    }

    /// The site settings; `None` until content has been synced.
    pub async fn site(&self) -> Result<Option<Site>, Error> {
        let row: Option<(String, String, String, String)> =
            sqlx::query_as("SELECT title, theme, values_json, updated_at FROM site WHERE id = 1")
                .fetch_optional(self.pool())
                .await?;
        row.map(|(title, theme, values, updated_at)| {
            Ok(Site {
                spec: SiteSpec {
                    title,
                    theme,
                    values: serde_json::from_str(&values)?,
                },
                updated_at,
            })
        })
        .transpose()
    }

    /// The site settings, which synced content always has.
    pub async fn site_config(&self) -> Result<Site, Error> {
        self.site().await?.ok_or(Error::Invariant("site missing"))
    }

    pub async fn profile(&self) -> Result<Profile, Error> {
        let row: (String, String, String, Option<String>, String, String) = sqlx::query_as(
            "SELECT name, headline, bio, email, links, updated_at FROM profile WHERE id = 1",
        )
        .fetch_optional(self.pool())
        .await?
        .ok_or(Error::Invariant("profile missing"))?;
        let (name, headline, bio, email, links, updated_at) = row;
        Ok(Profile {
            spec: ProfileSpec {
                name,
                headline,
                bio,
                email,
                links: serde_json::from_str(&links)?,
            },
            updated_at,
        })
    }

    /// The nav; `None` until this release has synced content, which is how
    /// `/readyz` tells a database an older release synced.
    pub async fn nav(&self) -> Result<Option<Nav>, Error> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT spec, updated_at FROM nav WHERE id = 1")
                .fetch_optional(self.pool())
                .await?;
        row.map(|(spec, updated_at)| {
            Ok(Nav {
                spec: serde_json::from_str(&spec)?,
                updated_at,
            })
        })
        .transpose()
    }

    /// Every page type, by name.
    pub async fn page_kinds(&self) -> Result<Vec<PageKind>, Error> {
        let rows: Vec<NamedRow> =
            sqlx::query_as("SELECT name, spec, updated_at FROM page_kinds ORDER BY name")
                .fetch_all(self.pool())
                .await?;
        rows.into_iter()
            .map(|row| {
                Ok(PageKind {
                    spec: serde_json::from_str(&row.spec)?,
                    name: row.name,
                    updated_at: row.updated_at,
                })
            })
            .collect()
    }

    /// Every collection, by name.
    pub async fn collections(&self) -> Result<Vec<Collection>, Error> {
        let rows: Vec<NamedRow> =
            sqlx::query_as("SELECT name, spec, updated_at FROM collections ORDER BY name")
                .fetch_all(self.pool())
                .await?;
        rows.into_iter()
            .map(|row| {
                Ok(Collection {
                    spec: serde_json::from_str(&row.spec)?,
                    name: row.name,
                    updated_at: row.updated_at,
                })
            })
            .collect()
    }

    /// Every page of every type, drafts included, by kind and name.
    pub async fn pages(&self) -> Result<Vec<Page>, Error> {
        let rows: Vec<PageRow> = sqlx::query_as(
            "SELECT kind, name, collection, metadata, spec, created_at, updated_at
             FROM pages ORDER BY kind, name",
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(page_from_row).collect()
    }

    /// A page, drafts included.
    pub async fn page(&self, kind: &str, name: &str) -> Result<Option<Page>, Error> {
        let row: Option<PageRow> = sqlx::query_as(
            "SELECT kind, name, collection, metadata, spec, created_at, updated_at
             FROM pages WHERE kind = ?1 AND name = ?2",
        )
        .bind(kind)
        .bind(name)
        .fetch_optional(self.pool())
        .await?;
        row.map(page_from_row).transpose()
    }

    /// A page's markdown.
    pub async fn content(&self, kind: &str, name: &str) -> Result<Option<String>, Error> {
        Ok(
            sqlx::query_scalar("SELECT content FROM pages WHERE kind = ?1 AND name = ?2")
                .bind(kind)
                .bind(name)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub async fn theme(&self, slug: &str) -> Result<Option<Theme>, Error> {
        let row: Option<ThemeRow> = sqlx::query_as(themes!(" WHERE slug = ?1"))
            .bind(slug)
            .fetch_optional(self.pool())
            .await?;
        row.map(theme_from_row).transpose()
    }

    /// Every theme, hidden ones included: those with an order first, lowest
    /// first, then the others by name.
    pub async fn themes(&self) -> Result<Vec<Theme>, Error> {
        let rows: Vec<ThemeRow> = sqlx::query_as(themes!(
            " ORDER BY CASE WHEN sort_order IS NULL THEN 1 ELSE 0 END, sort_order, slug"
        ))
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(theme_from_row).collect()
    }

    pub async fn sync_state(&self) -> Result<SyncState, Error> {
        let state: Option<SyncState> = sqlx::query_as(
            "SELECT revision, synced_at, last_attempt_at, last_error FROM sync_state WHERE id = 1",
        )
        .fetch_optional(self.pool())
        .await?;
        Ok(state.unwrap_or_default())
    }

    /// Records a sync that materialized `revision`; `None` for content read
    /// from a directory, which has no revision.
    pub async fn record_sync(&self, revision: Option<&str>) -> Result<(), Error> {
        sqlx::query(
            "INSERT INTO sync_state (id, revision, synced_at, last_attempt_at, last_error)
             VALUES (1, ?1, ?2, ?2, NULL)
             ON CONFLICT(id) DO UPDATE SET
                revision = excluded.revision,
                synced_at = excluded.synced_at,
                last_attempt_at = excluded.last_attempt_at,
                last_error = NULL",
        )
        .bind(revision)
        .bind(now_rfc3339())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Records an attempt that changed nothing: `error` says why it failed,
    /// `None` means the content was already current.
    pub async fn record_sync_attempt(&self, error: Option<&str>) -> Result<(), Error> {
        sqlx::query(
            "INSERT INTO sync_state (id, last_attempt_at, last_error) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET
                last_attempt_at = excluded.last_attempt_at,
                last_error = excluded.last_error",
        )
        .bind(now_rfc3339())
        .bind(error)
        .execute(self.pool())
        .await?;
        Ok(())
    }
}

/// Orders a collection's pages as its listing shows them: ascending by
/// `field`, pages without it last, then the most recently changed first.
pub fn order(pages: &mut [&Page], field: Option<&str>) {
    let key = |page: &Page| {
        field
            .and_then(|field| page.page.metadata.get(field))
            .cloned()
    };
    pages.sort_by(|a, b| {
        let by_field = match (key(a), key(b)) {
            (Some(x), Some(y)) => compare(&x, &y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        by_field
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.page.name.cmp(&b.page.name))
    });
}

fn compare(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x
            .as_i64()
            .cmp(&y.as_i64())
            .then_with(|| x.to_string().cmp(&y.to_string())),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        _ => Ordering::Equal,
    }
}

fn page_from_row(row: PageRow) -> Result<Page, Error> {
    Ok(Page {
        page: manifest::Page {
            kind: row.kind,
            name: row.name,
            collection: row.collection,
            metadata: serde_json::from_str(&row.metadata)?,
            spec: serde_json::from_str(&row.spec)?,
        },
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn theme_from_row(row: ThemeRow) -> Result<Theme, Error> {
    Ok(Theme {
        slug: row.slug,
        title: row.title,
        dark: row.dark,
        order: row.sort_order,
        hidden: row.hidden,
        colors: serde_json::from_str(&row.colors)?,
        updated_at: row.updated_at,
    })
}

/// Now, as the database and the API write every time: ISO-8601 UTC, to the
/// second.
pub fn now_rfc3339() -> String {
    rfc3339(chrono::Utc::now())
}

/// A time as [`now_rfc3339`] writes it.
pub fn rfc3339(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A Unix time, such as GitHub's rate-limit reset, as [`now_rfc3339`]
/// writes times; `None` when out of range.
pub fn from_unix(seconds: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(seconds, 0).map(rfc3339)
}

/// A time as GitHub writes a token's expiry, such as `2027-09-22 12:00:00
/// UTC` or `2027-09-22 12:00:00 +0000`, as [`now_rfc3339`] writes times.
pub fn from_github_time(text: &str) -> Option<String> {
    let text = text.trim();
    let at = match text.strip_suffix(" UTC") {
        Some(naive) => chrono::NaiveDateTime::parse_from_str(naive, "%Y-%m-%d %H:%M:%S")
            .ok()?
            .and_utc(),
        None => chrono::DateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S %z")
            .ok()?
            .to_utc(),
    };
    Some(rfc3339(at))
}

/// Whether `at`, as [`now_rfc3339`] writes times, is less than `days` away
/// or already past; a time that does not parse is not.
pub fn within_days(at: &str, days: i64) -> bool {
    chrono::DateTime::parse_from_rfc3339(at)
        .is_ok_and(|at| at.to_utc() - chrono::Utc::now() < chrono::TimeDelta::days(days))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index;
    use crate::testing::{self, TREE};

    async fn seeded() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let content = dir.path().join("content");
        testing::write(&content, &TREE);
        let draft = testing::ATLAS
            .replace("name: atlas", "name: hidden")
            .replace("title: Atlas", "title: Hidden")
            .replace("  highlight: 1\n", "")
            .replace("spec:\n", "spec:\n  draft: true\n");
        testing::write(
            &content,
            &[
                (
                    "projects/hidden.yaml",
                    draft.replace("atlas.md", "hidden.md").as_str(),
                ),
                ("projects/hidden.md", "Draft.\n"),
            ],
        );
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        index::sync(db.pool(), &content).await.unwrap();
        (dir, db)
    }

    #[test]
    fn reads_github_token_expiries() {
        assert_eq!(
            from_github_time("2027-09-22 12:00:00 UTC").as_deref(),
            Some("2027-09-22T12:00:00Z")
        );
        assert_eq!(
            from_github_time("2027-09-22 09:00:00 -0300").as_deref(),
            Some("2027-09-22T12:00:00Z")
        );
        assert!(from_github_time("next year").is_none());

        let in_days = |days| rfc3339(chrono::Utc::now() + chrono::TimeDelta::days(days));
        assert!(within_days(&in_days(29), 30));
        assert!(within_days(&in_days(-1), 30), "past");
        assert!(!within_days(&in_days(31), 30));
        assert!(!within_days("soon", 30));
    }

    #[tokio::test]
    async fn reads_what_the_indexer_wrote() {
        let (_dir, db) = seeded().await;
        let site = db.site_config().await.unwrap();
        assert_eq!(site.spec.title, "example.test");
        assert_eq!(site.spec.values["about"], "Design to platform.");
        let profile = db.profile().await.unwrap();
        assert_eq!(profile.spec.name, "Highlander Paiva");
        let nav = db.nav().await.unwrap().unwrap();
        assert_eq!(nav.spec.sections[0].name, "files");
        let kinds = db.page_kinds().await.unwrap();
        assert_eq!(kinds[0].spec.names.plural, "projects");
        let collections = db.collections().await.unwrap();
        assert_eq!(collections[0].spec.order.as_deref(), Some("highlight"));
        let theme = db.theme("nord").await.unwrap().unwrap();
        assert_eq!(theme.colors.bg.as_str(), "#2e3440");
        assert!(!theme.hidden);
        db.ping().await.unwrap();
    }

    #[tokio::test]
    async fn pages_keep_their_type_metadata_and_content() {
        let (_dir, db) = seeded().await;
        let pages = db.pages().await.unwrap();
        let names: Vec<(&str, &str)> = pages
            .iter()
            .map(|p| (p.page.kind.as_str(), p.page.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("Page", "about"),
                ("Page", "index"),
                ("Page", "not-found"),
                ("Page", "theme"),
                ("Project", "atlas"),
                ("Project", "hidden"),
            ]
        );
        let atlas = db.page("Project", "atlas").await.unwrap().unwrap();
        assert_eq!(atlas.page.collection.as_deref(), Some("projects"));
        assert_eq!(atlas.page.metadata["tagline"], "CubeSat ADCS in Rust");
        assert!(
            db.page("Project", "hidden")
                .await
                .unwrap()
                .unwrap()
                .page
                .spec
                .draft
        );
        assert_eq!(
            db.content("Project", "atlas").await.unwrap().as_deref(),
            Some(testing::ATLAS_MD)
        );
        assert!(
            db.content("Page", "not-found")
                .await
                .unwrap()
                .unwrap()
                .contains("start here")
        );
        assert_eq!(db.content("Page", "nope").await.unwrap(), None);
    }

    #[tokio::test]
    async fn orders_pages_as_their_collection_says() {
        let (_dir, db) = seeded().await;
        let pages = db.pages().await.unwrap();
        let mut members: Vec<&Page> = pages.iter().filter(|p| p.page.kind == "Project").collect();
        order(&mut members, Some("highlight"));
        let names: Vec<&str> = members.iter().map(|p| p.page.name.as_str()).collect();
        assert_eq!(names, ["atlas", "hidden"], "highlighted first");

        let mut a = pages[0].clone();
        let mut b = pages[1].clone();
        a.updated_at = "2026-01-01T00:00:00Z".to_owned();
        b.updated_at = "2026-02-01T00:00:00Z".to_owned();
        let mut both = vec![&a, &b];
        order(&mut both, None);
        assert_eq!(both[0].page.name, b.page.name, "newest first");
    }

    #[tokio::test]
    async fn orders_themes_by_order_then_name() {
        let dir = tempfile::tempdir().unwrap();
        let content = dir.path().join("content");
        testing::write(&content, &TREE);
        let theme = |name: &str, extra: &str| {
            testing::THEME
                .replace("name: nord", &format!("name: {name}"))
                .replace("  dark: true\n", &format!("  dark: true\n{extra}"))
        };
        testing::write(
            &content,
            &[
                ("themes/alpha.yaml", &theme("alpha", "")),
                ("themes/zeta.yaml", &theme("zeta", "  order: 1\n")),
                (
                    "themes/mid.yaml",
                    &theme("mid", "  order: 2\n  hidden: true\n"),
                ),
            ],
        );
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        index::sync(db.pool(), &content).await.unwrap();
        let names: Vec<String> = db
            .themes()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.slug)
            .collect();
        assert_eq!(names, ["zeta", "mid", "alpha", "nord"]);
        assert!(db.theme("mid").await.unwrap().unwrap().hidden);
    }

    #[tokio::test]
    async fn tracks_sync_state() {
        let (_dir, db) = seeded().await;
        assert_eq!(db.sync_state().await.unwrap(), SyncState::default());

        db.record_sync(Some("abc")).await.unwrap();
        let synced = db.sync_state().await.unwrap();
        assert_eq!(synced.revision.as_deref(), Some("abc"));
        assert!(synced.synced_at.is_some());
        assert!(synced.last_error.is_none());

        db.record_sync_attempt(Some("fetch failed")).await.unwrap();
        let failed = db.sync_state().await.unwrap();
        assert_eq!(failed.revision.as_deref(), Some("abc"));
        assert_eq!(failed.synced_at, synced.synced_at);
        assert_eq!(failed.last_error.as_deref(), Some("fetch failed"));

        db.record_sync_attempt(None).await.unwrap();
        assert!(db.sync_state().await.unwrap().last_error.is_none());
    }
}
