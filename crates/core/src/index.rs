use std::collections::HashSet;
use std::path::Path;

use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::Error;
pub use crate::api::SyncReport;
use crate::api::{ProfileSpec, ProjectSpec, SiteSpec, ThemeSpec};
use crate::manifest::{Kind, Manifest};
use crate::markdown::Markdown;
use crate::tree::Tree;

type Tx<'a> = sqlx::Transaction<'a, sqlx::Sqlite>;

/// Materializes the content tree into the database, in one transaction.
/// The whole tree is read and checked before anything is written, so a tree
/// with any problem leaves the previous state in place.
pub async fn sync(pool: &SqlitePool, content_dir: &Path) -> Result<SyncReport, Error> {
    let tree = Tree::read(content_dir)?;
    let markdown = Markdown::new();
    let mut tx = pool.begin().await?;
    let mut report = SyncReport::default();
    let mut projects = HashSet::new();
    let mut themes = HashSet::new();
    for file in &tree.files {
        let source_hash = hash(&file.bytes);
        match &file.manifest {
            Manifest::Site(spec) => {
                report.site_updated = upsert_site(&mut tx, spec, &source_hash).await?;
            }
            Manifest::Profile(spec) => {
                report.profile_updated =
                    upsert_profile(&mut tx, spec, &source_hash, &markdown).await?;
            }
            Manifest::Project { name, spec } => {
                projects.insert(name.clone());
                if upsert_project(&mut tx, name, spec, &source_hash, &markdown).await? {
                    report.projects_upserted += 1;
                } else {
                    report.projects_skipped += 1;
                }
            }
            Manifest::Theme { name, spec } => {
                themes.insert(name.clone());
                if upsert_theme(&mut tx, name, spec, &source_hash).await? {
                    report.themes_upserted += 1;
                } else {
                    report.themes_skipped += 1;
                }
            }
        }
    }
    report.projects_deleted = delete_missing(&mut tx, Kind::Project, &projects).await?;
    report.themes_deleted = delete_missing(&mut tx, Kind::Theme, &themes).await?;

    tx.commit().await?;
    Ok(report)
}

/// Whether the row already holds a file with this hash, which leaves it as is.
async fn unchanged(
    tx: &mut Tx<'_>,
    sql: &'static str,
    key: Option<&str>,
    source_hash: &str,
) -> Result<bool, Error> {
    let mut query = sqlx::query_scalar::<_, String>(sql);
    if let Some(key) = key {
        query = query.bind(key);
    }
    let existing = query.fetch_optional(&mut **tx).await?;
    Ok(existing.as_deref() == Some(source_hash))
}

async fn upsert_site(tx: &mut Tx<'_>, spec: &SiteSpec, source_hash: &str) -> Result<bool, Error> {
    if unchanged(
        tx,
        "SELECT source_hash FROM site WHERE id = 1",
        None,
        source_hash,
    )
    .await?
    {
        return Ok(false);
    }
    let SiteSpec {
        title,
        theme,
        banner,
        descriptions,
        blog,
    } = spec;
    sqlx::query(
        "INSERT INTO site (
            id, title, theme, banner, descriptions, blog_enabled, source_hash, updated_at
        ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(id) DO UPDATE SET
            title = excluded.title,
            theme = excluded.theme,
            banner = excluded.banner,
            descriptions = excluded.descriptions,
            blog_enabled = excluded.blog_enabled,
            source_hash = excluded.source_hash,
            updated_at = excluded.updated_at",
    )
    .bind(title)
    .bind(theme)
    .bind(banner.as_ref().map(serde_json::to_string).transpose()?)
    .bind(serde_json::to_string(descriptions)?)
    .bind(blog.enabled)
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
    let sql = "SELECT source_hash FROM themes WHERE slug = ?1";
    if unchanged(tx, sql, Some(slug), source_hash).await? {
        return Ok(false);
    }
    let ThemeSpec {
        title,
        dark,
        colors,
    } = spec;
    sqlx::query(
        "INSERT INTO themes (slug, title, dark, colors, source_hash, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(slug) DO UPDATE SET
            title = excluded.title,
            dark = excluded.dark,
            colors = excluded.colors,
            source_hash = excluded.source_hash,
            updated_at = excluded.updated_at",
    )
    .bind(slug)
    .bind(title)
    .bind(dark)
    .bind(serde_json::to_string(colors)?)
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
    markdown: &Markdown,
) -> Result<bool, Error> {
    let sql = "SELECT source_hash FROM profile WHERE id = 1";
    if unchanged(tx, sql, None, source_hash).await? {
        return Ok(false);
    }
    let ProfileSpec {
        name,
        headline,
        bio,
        email,
        links,
        body,
    } = spec;
    sqlx::query(
        "INSERT INTO profile (
            id, name, headline, bio, about_source, about_html, about_text,
            email, links, source_hash, updated_at
        ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            headline = excluded.headline,
            bio = excluded.bio,
            about_source = excluded.about_source,
            about_html = excluded.about_html,
            about_text = excluded.about_text,
            email = excluded.email,
            links = excluded.links,
            source_hash = excluded.source_hash,
            updated_at = excluded.updated_at",
    )
    .bind(name)
    .bind(headline)
    .bind(bio)
    .bind(body)
    .bind(markdown.html(body))
    .bind(Markdown::text(body))
    .bind(email)
    .bind(serde_json::to_string(links)?)
    .bind(source_hash)
    .bind(now_rfc3339())
    .execute(&mut **tx)
    .await?;
    Ok(true)
}

async fn upsert_project(
    tx: &mut Tx<'_>,
    slug: &str,
    spec: &ProjectSpec,
    source_hash: &str,
    markdown: &Markdown,
) -> Result<bool, Error> {
    let sql = "SELECT source_hash FROM projects WHERE slug = ?1";
    if unchanged(tx, sql, Some(slug), source_hash).await? {
        return Ok(false);
    }
    let ProjectSpec {
        title,
        tagline,
        status,
        draft,
        highlight,
        tags,
        links,
        github,
        assets,
        body,
    } = spec;

    sqlx::query(
        "INSERT INTO projects (
            slug, title, tagline, status, draft, highlight, tags, links, github_repo,
            body_source, body_html, body_text, source_hash, created_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14
        )
        ON CONFLICT(slug) DO UPDATE SET
            title = excluded.title,
            tagline = excluded.tagline,
            status = excluded.status,
            draft = excluded.draft,
            highlight = excluded.highlight,
            tags = excluded.tags,
            links = excluded.links,
            github_repo = excluded.github_repo,
            body_source = excluded.body_source,
            body_html = excluded.body_html,
            body_text = excluded.body_text,
            source_hash = excluded.source_hash,
            created_at = projects.created_at,
            updated_at = excluded.updated_at",
    )
    .bind(slug)
    .bind(title)
    .bind(tagline)
    .bind(status.as_str())
    .bind(draft)
    .bind(highlight)
    .bind(serde_json::to_string(tags)?)
    .bind(serde_json::to_string(links)?)
    .bind(github)
    .bind(body)
    .bind(markdown.html(body))
    .bind(Markdown::text(body))
    .bind(source_hash)
    .bind(now_rfc3339())
    .execute(&mut **tx)
    .await?;

    sqlx::query("DELETE FROM project_assets WHERE project_slug = ?1")
        .bind(slug)
        .execute(&mut **tx)
        .await?;

    for asset in assets {
        sqlx::query(
            "INSERT INTO project_assets (project_slug, path, caption, width, height, hash, derivatives)
             VALUES (?1, ?2, ?3, 0, 0, '', '[]')",
        )
        .bind(slug)
        .bind(&asset.path)
        .bind(&asset.caption)
        .execute(&mut **tx)
        .await?;
    }

    Ok(true)
}

/// Removes rows of a named kind whose files are gone.
async fn delete_missing(tx: &mut Tx<'_>, kind: Kind, seen: &HashSet<String>) -> Result<u32, Error> {
    let (select, delete) = match kind {
        Kind::Project => (
            "SELECT slug FROM projects",
            "DELETE FROM projects WHERE slug = ?1",
        ),
        Kind::Theme => (
            "SELECT slug FROM themes",
            "DELETE FROM themes WHERE slug = ?1",
        ),
        Kind::Profile | Kind::Site => return Err(Error::Invariant("singletons are not pruned")),
    };
    let slugs: Vec<String> = sqlx::query_scalar(select).fetch_all(&mut **tx).await?;
    let mut deleted = 0;
    for slug in slugs.iter().filter(|slug| !seen.contains(*slug)) {
        sqlx::query(delete).bind(slug).execute(&mut **tx).await?;
        deleted += 1;
    }
    Ok(deleted)
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
    use crate::testing::{SITE, THEME};

    const PROFILE: &str = "\
---
kind: Profile
name: Highlander Paiva
headline: senior platform engineer
bio: writes software.
email: contact@hvpaiva.dev
links:
  github: https://github.com/hvpaiva
---
Design to platform.
";

    const PROJECT: &str = "\
---
kind: Project
title: Atlas
tagline: CubeSat ADCS in Rust
status: active
highlight: 1
tags: [rust, aerospace]
links:
  repo: https://github.com/hvpaiva/atlas
github: hvpaiva/atlas
assets:
  - path: atlas/overview.png
    caption: pipeline
---

Context and **decisions**.
";

    async fn setup(dir: &Path) -> Db {
        crate::testing::write_site(dir);
        fs::write(dir.join("profile.md"), PROFILE).unwrap();
        fs::create_dir(dir.join("projects")).unwrap();
        fs::write(dir.join("projects/atlas.md"), PROJECT).unwrap();
        Db::open(dir.join("hldr.db")).await.unwrap()
    }

    async fn scalar(db: &Db, sql: &'static str) -> String {
        sqlx::query_scalar(sql).fetch_one(db.pool()).await.unwrap()
    }

    #[tokio::test]
    async fn indexes_site_profile_and_project() {
        let dir = tempfile::tempdir().unwrap();
        let db = setup(dir.path()).await;
        let report = sync(db.pool(), dir.path()).await.unwrap();
        assert!(report.profile_updated);
        assert_eq!(report.projects_upserted, 1);
        assert_eq!(report.projects_skipped, 0);

        let title = scalar(&db, "SELECT title FROM projects WHERE slug = 'atlas'").await;
        assert_eq!(title, "Atlas");

        let html = scalar(&db, "SELECT body_html FROM projects WHERE slug = 'atlas'").await;
        assert!(html.contains("<strong>decisions</strong>"), "{html}");

        let caption = scalar(
            &db,
            "SELECT caption FROM project_assets WHERE project_slug = 'atlas'",
        )
        .await;
        assert_eq!(caption, "pipeline");

        let about = scalar(&db, "SELECT about_source FROM profile").await;
        assert_eq!(about, "Design to platform.\n");

        assert!(!db.blog_enabled().await.unwrap());
        let site = db.site_config().await.unwrap();
        assert_eq!(site.title, "example.test");
        assert_eq!(site.theme, "nord");
        let theme = db.theme("nord").await.unwrap().unwrap();
        assert_eq!(theme.colors.bg.as_str(), "#2e3440");
    }

    #[tokio::test]
    async fn second_pass_skips_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let db = setup(dir.path()).await;
        sync(db.pool(), dir.path()).await.unwrap();
        let report = sync(db.pool(), dir.path()).await.unwrap();
        assert!(!report.site_updated);
        assert!(!report.profile_updated);
        assert_eq!(report.projects_upserted, 0);
        assert_eq!(report.projects_skipped, 1);
        assert_eq!(report.projects_deleted, 0);
    }

    #[tokio::test]
    async fn site_manifest_drives_the_site() {
        let dir = tempfile::tempdir().unwrap();
        let db = setup(dir.path()).await;
        sync(db.pool(), dir.path()).await.unwrap();
        fs::write(dir.path().join("site.yaml"), SITE.replace("false", "true")).unwrap();
        let report = sync(db.pool(), dir.path()).await.unwrap();
        assert!(report.site_updated);
        assert!(db.blog_enabled().await.unwrap());
    }

    #[tokio::test]
    async fn removes_projects_deleted_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let db = setup(dir.path()).await;
        sync(db.pool(), dir.path()).await.unwrap();
        fs::remove_file(dir.path().join("projects/atlas.md")).unwrap();
        let report = sync(db.pool(), dir.path()).await.unwrap();
        assert_eq!(report.projects_deleted, 1);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn preserves_created_at() {
        let dir = tempfile::tempdir().unwrap();
        let db = setup(dir.path()).await;
        sync(db.pool(), dir.path()).await.unwrap();
        sqlx::query("UPDATE projects SET created_at = '2020-01-01T00:00:00Z'")
            .execute(db.pool())
            .await
            .unwrap();
        fs::write(
            dir.path().join("projects/atlas.md"),
            PROJECT.replace("CubeSat ADCS in Rust", "updated tagline"),
        )
        .unwrap();
        sync(db.pool(), dir.path()).await.unwrap();
        let (created_at, tagline): (String, String) =
            sqlx::query_as("SELECT created_at, tagline FROM projects WHERE slug = 'atlas'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(created_at, "2020-01-01T00:00:00Z");
        assert_eq!(tagline, "updated tagline");
    }

    #[tokio::test]
    async fn a_bad_file_leaves_the_previous_state() {
        let dir = tempfile::tempdir().unwrap();
        let db = setup(dir.path()).await;
        sync(db.pool(), dir.path()).await.unwrap();

        fs::write(dir.path().join("site.yaml"), SITE.replace("false", "true")).unwrap();
        fs::write(
            dir.path().join("projects/broken.md"),
            PROJECT.replace("kind: Project", "kind: Site"),
        )
        .unwrap();
        let err = sync(db.pool(), dir.path()).await.unwrap_err();
        assert!(err.to_string().contains("projects/broken.md"), "{err}");

        assert!(!db.blog_enabled().await.unwrap());
        assert_eq!(db.project_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn requires_the_site_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let db = setup(dir.path()).await;
        fs::remove_file(dir.path().join("site.yaml")).unwrap();
        let err = sync(db.pool(), dir.path()).await.unwrap_err();
        assert!(err.to_string().contains("site.yaml"), "{err}");
    }

    #[tokio::test]
    async fn indexes_and_prunes_themes() {
        let dir = tempfile::tempdir().unwrap();
        let db = setup(dir.path()).await;
        fs::write(
            dir.path().join("themes/dawn.yaml"),
            THEME
                .replace("title: Nord", "title: Dawn")
                .replace("dark: true", "dark: false"),
        )
        .unwrap();
        let report = sync(db.pool(), dir.path()).await.unwrap();
        assert_eq!(report.themes_upserted, 2);
        let names: Vec<String> = db
            .themes()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.slug)
            .collect();
        assert_eq!(names, ["dawn", "nord"]);

        fs::remove_file(dir.path().join("themes/dawn.yaml")).unwrap();
        let report = sync(db.pool(), dir.path()).await.unwrap();
        assert_eq!(report.themes_deleted, 1);
        assert_eq!(report.themes_skipped, 1);
        assert!(db.theme("dawn").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_default_theme_must_exist() {
        let dir = tempfile::tempdir().unwrap();
        let db = setup(dir.path()).await;
        sync(db.pool(), dir.path()).await.unwrap();

        fs::remove_file(dir.path().join("themes/nord.yaml")).unwrap();
        let err = sync(db.pool(), dir.path()).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("theme nord has no themes/nord.yaml"),
            "{err}"
        );
        assert!(db.theme("nord").await.unwrap().is_some());

        fs::write(dir.path().join("themes/nord.yaml"), THEME).unwrap();
        fs::write(
            dir.path().join("site.yaml"),
            SITE.replace("theme: nord", "theme: dusk"),
        )
        .unwrap();
        let err = sync(db.pool(), dir.path()).await.unwrap_err();
        assert!(err.to_string().contains("theme dusk"), "{err}");
    }
}
