use std::collections::HashSet;
use std::path::Path;

use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::Error;
pub use crate::api::SyncReport;
use crate::manifest::{Kind, Manifest, Page};
use crate::spec::{NavSpec, ProfileSpec, SiteSpec, ThemeSpec};
use crate::tree::Tree;

type Tx<'a> = sqlx::Transaction<'a, sqlx::Sqlite>;

/// Materializes the content tree into the database, in one transaction.
/// The whole tree is read and checked before anything is written, so a tree
/// with any problem leaves the previous state in place.
pub async fn sync(pool: &SqlitePool, content_dir: &Path) -> Result<SyncReport, Error> {
    let tree = Tree::read(content_dir)?;
    let mut tx = pool.begin().await?;
    let mut report = SyncReport::default();
    let mut kinds = HashSet::new();
    let mut collections = HashSet::new();
    let mut themes = HashSet::new();
    let mut pages = HashSet::new();
    for file in &tree.files {
        let source_hash = hash(&file.source());
        let written = match &file.manifest {
            Manifest::Site(spec) => upsert_site(&mut tx, spec, &source_hash).await?,
            Manifest::Profile(spec) => upsert_profile(&mut tx, spec, &source_hash).await?,
            Manifest::Nav(spec) => upsert_nav(&mut tx, spec, &source_hash).await?,
            Manifest::PageKind { name, spec } => {
                kinds.insert(name.clone());
                let spec = serde_json::to_string(spec)?;
                upsert_named(&mut tx, Named::PageKinds, name, &spec, &source_hash).await?
            }
            Manifest::Collection { name, spec } => {
                collections.insert(name.clone());
                let spec = serde_json::to_string(spec)?;
                upsert_named(&mut tx, Named::Collections, name, &spec, &source_hash).await?
            }
            Manifest::Theme { name, spec } => {
                themes.insert(name.clone());
                upsert_theme(&mut tx, name, spec, &source_hash).await?
            }
            Manifest::Page(page) => {
                pages.insert((page.kind.clone(), page.name.clone()));
                let content = file.markdown().unwrap_or_default();
                upsert_page(&mut tx, page, content, &source_hash).await?
            }
        };
        report.count(file.manifest.kind(), written);
    }
    let deleted = delete_missing(&mut tx, Named::PageKinds, &kinds).await?;
    report.deleted(Kind::PageKind.as_str(), deleted);
    let deleted = delete_missing(&mut tx, Named::Collections, &collections).await?;
    report.deleted(Kind::Collection.as_str(), deleted);
    let deleted = delete_missing(&mut tx, Named::Themes, &themes).await?;
    report.deleted(Kind::Theme.as_str(), deleted);
    delete_missing_pages(&mut tx, &pages, &mut report).await?;

    tx.commit().await?;
    Ok(report)
}

/// Whether the row already holds a file with this hash, which leaves it as is.
async fn unchanged(
    tx: &mut Tx<'_>,
    sql: &'static str,
    key: &[&str],
    source_hash: &str,
) -> Result<bool, Error> {
    let mut query = sqlx::query_scalar::<_, String>(sql);
    for part in key {
        query = query.bind(*part);
    }
    let existing = query.fetch_optional(&mut **tx).await?;
    Ok(existing.as_deref() == Some(source_hash))
}

/// Writes the columns this release reads. The legacy columns get values
/// only on a fresh database, where no older release reads them.
async fn upsert_site(tx: &mut Tx<'_>, spec: &SiteSpec, source_hash: &str) -> Result<bool, Error> {
    let sql = "SELECT source_hash FROM site WHERE id = 1";
    if unchanged(tx, sql, &[], source_hash).await? {
        return Ok(false);
    }
    let SiteSpec {
        title,
        theme,
        values,
    } = spec;
    sqlx::query(
        "INSERT INTO site (
            id, title, theme, values_json, banner, descriptions, blog_enabled,
            source_hash, updated_at
        ) VALUES (1, ?1, ?2, ?3, NULL, '{\"projects\":\"\",\"themes\":\"\"}', 0, ?4, ?5)
        ON CONFLICT(id) DO UPDATE SET
            title = excluded.title,
            theme = excluded.theme,
            values_json = excluded.values_json,
            source_hash = excluded.source_hash,
            updated_at = excluded.updated_at",
    )
    .bind(title)
    .bind(theme)
    .bind(serde_json::to_string(values)?)
    .bind(source_hash)
    .bind(now_rfc3339())
    .execute(&mut **tx)
    .await?;
    Ok(true)
}

async fn upsert_profile(
    tx: &mut Tx<'_>,
    spec: &ProfileSpec,
    source_hash: &str,
) -> Result<bool, Error> {
    let sql = "SELECT source_hash FROM profile WHERE id = 1";
    if unchanged(tx, sql, &[], source_hash).await? {
        return Ok(false);
    }
    let ProfileSpec {
        name,
        headline,
        bio,
        email,
        links,
    } = spec;
    sqlx::query(
        "INSERT INTO profile (id, name, headline, bio, email, links, source_hash, updated_at)
        VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            headline = excluded.headline,
            bio = excluded.bio,
            email = excluded.email,
            links = excluded.links,
            source_hash = excluded.source_hash,
            updated_at = excluded.updated_at",
    )
    .bind(name)
    .bind(headline)
    .bind(bio)
    .bind(email)
    .bind(serde_json::to_string(links)?)
    .bind(source_hash)
    .bind(now_rfc3339())
    .execute(&mut **tx)
    .await?;
    Ok(true)
}

async fn upsert_nav(tx: &mut Tx<'_>, spec: &NavSpec, source_hash: &str) -> Result<bool, Error> {
    let sql = "SELECT source_hash FROM nav WHERE id = 1";
    if unchanged(tx, sql, &[], source_hash).await? {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO nav (id, spec, source_hash, updated_at) VALUES (1, ?1, ?2, ?3)
        ON CONFLICT(id) DO UPDATE SET
            spec = excluded.spec,
            source_hash = excluded.source_hash,
            updated_at = excluded.updated_at",
    )
    .bind(serde_json::to_string(spec)?)
    .bind(source_hash)
    .bind(now_rfc3339())
    .execute(&mut **tx)
    .await?;
    Ok(true)
}

/// A table of named resources whose rows hold a spec as JSON, or themes.
#[derive(Clone, Copy)]
enum Named {
    PageKinds,
    Collections,
    Themes,
}

impl Named {
    fn hash(self) -> &'static str {
        match self {
            Self::PageKinds => "SELECT source_hash FROM page_kinds WHERE name = ?1",
            Self::Collections => "SELECT source_hash FROM collections WHERE name = ?1",
            Self::Themes => "SELECT source_hash FROM themes WHERE slug = ?1",
        }
    }

    fn upsert(self) -> &'static str {
        match self {
            Self::PageKinds => {
                "INSERT INTO page_kinds (name, spec, source_hash, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(name) DO UPDATE SET
                    spec = excluded.spec,
                    source_hash = excluded.source_hash,
                    updated_at = excluded.updated_at"
            }
            Self::Collections => {
                "INSERT INTO collections (name, spec, source_hash, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(name) DO UPDATE SET
                    spec = excluded.spec,
                    source_hash = excluded.source_hash,
                    updated_at = excluded.updated_at"
            }
            Self::Themes => unreachable!("themes have columns of their own"),
        }
    }

    fn names(self) -> &'static str {
        match self {
            Self::PageKinds => "SELECT name FROM page_kinds",
            Self::Collections => "SELECT name FROM collections",
            Self::Themes => "SELECT slug FROM themes",
        }
    }

    fn delete(self) -> &'static str {
        match self {
            Self::PageKinds => "DELETE FROM page_kinds WHERE name = ?1",
            Self::Collections => "DELETE FROM collections WHERE name = ?1",
            Self::Themes => "DELETE FROM themes WHERE slug = ?1",
        }
    }
}

async fn upsert_named(
    tx: &mut Tx<'_>,
    table: Named,
    name: &str,
    spec: &str,
    source_hash: &str,
) -> Result<bool, Error> {
    if unchanged(tx, table.hash(), &[name], source_hash).await? {
        return Ok(false);
    }
    sqlx::query(table.upsert())
        .bind(name)
        .bind(spec)
        .bind(source_hash)
        .bind(now_rfc3339())
        .execute(&mut **tx)
        .await?;
    Ok(true)
}

async fn upsert_theme(
    tx: &mut Tx<'_>,
    slug: &str,
    spec: &ThemeSpec,
    source_hash: &str,
) -> Result<bool, Error> {
    if unchanged(tx, Named::Themes.hash(), &[slug], source_hash).await? {
        return Ok(false);
    }
    let ThemeSpec {
        title,
        dark,
        order,
        hidden,
        colors,
    } = spec;
    sqlx::query(
        "INSERT INTO themes (slug, title, dark, sort_order, hidden, colors, source_hash, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(slug) DO UPDATE SET
            title = excluded.title,
            dark = excluded.dark,
            sort_order = excluded.sort_order,
            hidden = excluded.hidden,
            colors = excluded.colors,
            source_hash = excluded.source_hash,
            updated_at = excluded.updated_at",
    )
    .bind(slug)
    .bind(title)
    .bind(dark)
    .bind(order)
    .bind(hidden)
    .bind(serde_json::to_string(colors)?)
    .bind(source_hash)
    .bind(now_rfc3339())
    .execute(&mut **tx)
    .await?;
    Ok(true)
}

async fn upsert_page(
    tx: &mut Tx<'_>,
    page: &Page,
    content: &str,
    source_hash: &str,
) -> Result<bool, Error> {
    let sql = "SELECT source_hash FROM pages WHERE kind = ?1 AND name = ?2";
    if unchanged(tx, sql, &[&page.kind, &page.name], source_hash).await? {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO pages (
            kind, name, collection, metadata, spec, content, source_hash, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
        ON CONFLICT(kind, name) DO UPDATE SET
            collection = excluded.collection,
            metadata = excluded.metadata,
            spec = excluded.spec,
            content = excluded.content,
            source_hash = excluded.source_hash,
            created_at = pages.created_at,
            updated_at = excluded.updated_at",
    )
    .bind(&page.kind)
    .bind(&page.name)
    .bind(&page.collection)
    .bind(serde_json::to_string(&page.metadata)?)
    .bind(serde_json::to_string(&page.spec)?)
    .bind(content)
    .bind(source_hash)
    .bind(now_rfc3339())
    .execute(&mut **tx)
    .await?;
    Ok(true)
}

/// Removes rows of a named kind whose files are gone.
async fn delete_missing(
    tx: &mut Tx<'_>,
    table: Named,
    seen: &HashSet<String>,
) -> Result<u32, Error> {
    let names: Vec<String> = sqlx::query_scalar(table.names())
        .fetch_all(&mut **tx)
        .await?;
    let mut deleted = 0;
    for name in names.iter().filter(|name| !seen.contains(*name)) {
        sqlx::query(table.delete())
            .bind(name)
            .execute(&mut **tx)
            .await?;
        deleted += 1;
    }
    Ok(deleted)
}

async fn delete_missing_pages(
    tx: &mut Tx<'_>,
    seen: &HashSet<(String, String)>,
    report: &mut SyncReport,
) -> Result<(), Error> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT kind, name FROM pages")
        .fetch_all(&mut **tx)
        .await?;
    for row in rows.into_iter().filter(|row| !seen.contains(row)) {
        sqlx::query("DELETE FROM pages WHERE kind = ?1 AND name = ?2")
            .bind(&row.0)
            .bind(&row.1)
            .execute(&mut **tx)
            .await?;
        report.deleted(&row.0, 1);
    }
    Ok(())
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::db::Db;
    use crate::testing::{self, TREE};

    async fn setup() -> (tempfile::TempDir, std::path::PathBuf, Db) {
        let dir = tempfile::tempdir().unwrap();
        let content = dir.path().join("content");
        testing::write(&content, &TREE);
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        (dir, content, db)
    }

    async fn scalar(db: &Db, sql: &'static str) -> String {
        sqlx::query_scalar(sql).fetch_one(db.pool()).await.unwrap()
    }

    #[tokio::test]
    async fn indexes_every_kind() {
        let (_dir, content, db) = setup().await;
        let report = sync(db.pool(), &content).await.unwrap();
        for (kind, upserted) in [
            ("Site", 1),
            ("Profile", 1),
            ("Nav", 1),
            ("PageKind", 1),
            ("Collection", 1),
            ("Theme", 1),
            ("Page", 4),
            ("Project", 1),
        ] {
            assert_eq!(report.get(kind).upserted, upserted, "{kind}");
        }
        let about = scalar(&db, "SELECT content FROM pages WHERE name = 'about'").await;
        assert_eq!(about, testing::ABOUT_MD);
        let tagline = scalar(
            &db,
            "SELECT json_extract(metadata, '$.tagline') FROM pages WHERE name = 'atlas'",
        )
        .await;
        assert_eq!(tagline, "CubeSat ADCS in Rust");
    }

    #[tokio::test]
    async fn second_pass_skips_unchanged() {
        let (_dir, content, db) = setup().await;
        sync(db.pool(), &content).await.unwrap();
        let report = sync(db.pool(), &content).await.unwrap();
        assert!(
            report
                .kinds
                .values()
                .all(|k| k.upserted == 0 && k.deleted == 0),
            "{report:?}"
        );
        assert_eq!(report.get("Page").skipped, 4);
    }

    #[tokio::test]
    async fn a_changed_content_file_rewrites_its_page() {
        let (_dir, content, db) = setup().await;
        sync(db.pool(), &content).await.unwrap();
        fs::write(content.join("projects/atlas.md"), "## Changed\n").unwrap();
        let report = sync(db.pool(), &content).await.unwrap();
        assert_eq!(report.get("Project").upserted, 1);
        assert_eq!(report.get("Page").upserted, 0);
        let body = scalar(&db, "SELECT content FROM pages WHERE name = 'atlas'").await;
        assert_eq!(body, "## Changed\n");
    }

    #[tokio::test]
    async fn removes_pages_deleted_from_disk() {
        let (_dir, content, db) = setup().await;
        sync(db.pool(), &content).await.unwrap();
        fs::remove_file(content.join("projects/atlas.yaml")).unwrap();
        fs::remove_file(content.join("projects/atlas.md")).unwrap();
        let report = sync(db.pool(), &content).await.unwrap();
        assert_eq!(report.get("Project").deleted, 1);
        assert!(db.page("Project", "atlas").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn preserves_created_at() {
        let (_dir, content, db) = setup().await;
        sync(db.pool(), &content).await.unwrap();
        sqlx::query("UPDATE pages SET created_at = '2020-01-01T00:00:00Z'")
            .execute(db.pool())
            .await
            .unwrap();
        fs::write(
            content.join("projects/atlas.yaml"),
            testing::ATLAS.replace("CubeSat ADCS in Rust", "updated tagline"),
        )
        .unwrap();
        sync(db.pool(), &content).await.unwrap();
        let page = db.page("Project", "atlas").await.unwrap().unwrap();
        assert_eq!(page.created_at, "2020-01-01T00:00:00Z");
        assert_eq!(page.page.metadata["tagline"], "updated tagline");
    }

    #[tokio::test]
    async fn a_bad_file_leaves_the_previous_state() {
        let (_dir, content, db) = setup().await;
        sync(db.pool(), &content).await.unwrap();

        fs::write(
            content.join("site.yaml"),
            testing::SITE.replace("example.test", "changed.test"),
        )
        .unwrap();
        fs::write(content.join("projects/broken.yaml"), "kind: Site\n").unwrap();
        let err = sync(db.pool(), &content).await.unwrap_err();
        assert!(err.to_string().contains("projects/broken.yaml"), "{err}");
        assert_eq!(db.site_config().await.unwrap().spec.title, "example.test");
    }

    #[tokio::test]
    async fn keeps_what_the_previous_release_reads() {
        let (_dir, content, db) = setup().await;
        sqlx::query(
            "INSERT INTO site (id, title, theme, banner, descriptions, blog_enabled, source_hash, updated_at)
             VALUES (1, 'old', 'nord', NULL, '{\"projects\":\"P\",\"themes\":\"T\"}', 1, 'h', 't')",
        )
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO profile (id, name, headline, bio, about_source, email, links, source_hash, updated_at)
             VALUES (1, 'n', 'h', 'b', 'the about page', NULL, '{}', 'h', 't')",
        )
        .execute(db.pool())
        .await
        .unwrap();
        sync(db.pool(), &content).await.unwrap();
        let legacy: (String, bool, String) = sqlx::query_as(
            "SELECT descriptions, blog_enabled, (SELECT about_source FROM profile) FROM site",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(legacy.0, "{\"projects\":\"P\",\"themes\":\"T\"}");
        assert!(legacy.1);
        assert_eq!(legacy.2, "the about page");
        assert_eq!(db.site_config().await.unwrap().spec.title, "example.test");
    }

    #[tokio::test]
    async fn indexes_and_prunes_themes() {
        let (_dir, content, db) = setup().await;
        fs::write(
            content.join("themes/dawn.yaml"),
            testing::THEME
                .replace("name: nord", "name: dawn")
                .replace("title: Nord", "title: Dawn"),
        )
        .unwrap();
        let report = sync(db.pool(), &content).await.unwrap();
        assert_eq!(report.get("Theme").upserted, 2);
        fs::remove_file(content.join("themes/dawn.yaml")).unwrap();
        let report = sync(db.pool(), &content).await.unwrap();
        assert_eq!(report.get("Theme").deleted, 1);
        assert_eq!(report.get("Theme").skipped, 1);
        assert!(db.theme("dawn").await.unwrap().is_none());
    }
}
