use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::Error;
pub use crate::api::SyncReport;
use crate::api::{ProfileSpec, ProjectSpec, SiteSpec, ThemeSpec};
use crate::manifest::{self, Kind, Manifest};
use crate::markdown::Markdown;

type Tx<'a> = sqlx::Transaction<'a, sqlx::Sqlite>;

/// Materializes the content tree into the database, in one transaction:
/// a tree that fails to parse leaves the previous state in place.
pub async fn sync(pool: &SqlitePool, content_dir: &Path) -> Result<SyncReport, Error> {
    if !content_dir.is_dir() {
        return Err(Error::file(content_dir, "content directory does not exist"));
    }

    let markdown = Markdown::new();
    let mut tx = pool.begin().await?;
    let mut report = SyncReport {
        site_updated: sync_site(&mut tx, content_dir).await?,
        profile_updated: sync_profile(&mut tx, content_dir, &markdown).await?,
        ..SyncReport::default()
    };
    let seen = sync_projects(&mut tx, content_dir, &markdown, &mut report).await?;
    report.projects_deleted = delete_missing(&mut tx, Kind::Project, &seen).await?;
    let seen = sync_themes(&mut tx, content_dir, &mut report).await?;
    report.themes_deleted = delete_missing(&mut tx, Kind::Theme, &seen).await?;
    check_default_theme(&mut tx).await?;

    tx.commit().await?;
    Ok(report)
}

/// Reads a content file; `rel` is relative to the content root.
fn read(content_dir: &Path, rel: &Path) -> Result<Vec<u8>, Error> {
    fs::read(content_dir.join(rel)).map_err(|err| Error::file(rel, err.to_string()))
}

/// Files of one named kind, relative to the content root, in name order.
fn listed(content_dir: &Path, kind: Kind) -> Result<Vec<PathBuf>, Error> {
    let (dir, file) = kind
        .path_template()
        .split_once('/')
        .ok_or(Error::Invariant("named kinds live in a directory"))?;
    let extension = file.rsplit_once('.').map_or("", |(_, ext)| ext);
    let full = content_dir.join(dir);
    if !full.exists() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(&full)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == extension))
        .filter_map(|path| path.file_name().map(|name| Path::new(dir).join(name)))
        .collect();
    entries.sort();
    Ok(entries)
}

/// Name a file under a kind's directory holds, registered in `seen`.
fn claim(rel: &Path, seen: &mut HashSet<String>) -> Result<String, Error> {
    let name = manifest::locate(rel)?
        .and_then(|location| location.name)
        .ok_or_else(|| Error::file(rel, "not a named resource"))?;
    if !seen.insert(name.clone()) {
        return Err(Error::file(rel, "duplicate name"));
    }
    Ok(name)
}

async fn sync_site(tx: &mut Tx<'_>, content_dir: &Path) -> Result<bool, Error> {
    let rel = PathBuf::from(manifest::path(Kind::Site, None));
    let bytes = read(content_dir, &rel)?;
    let source_hash = hash(&bytes);
    let existing: Option<String> = sqlx::query_scalar("SELECT source_hash FROM site WHERE id = 1")
        .fetch_optional(&mut **tx)
        .await?;
    if existing.as_deref() == Some(source_hash.as_str()) {
        return Ok(false);
    }

    let Manifest::Site(SiteSpec {
        title,
        theme,
        banner,
        descriptions,
        blog,
    }) = manifest::parse(&rel, &bytes)?
    else {
        return Err(Error::file(&rel, "not a site manifest"));
    };
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
    .bind(&title)
    .bind(&theme)
    .bind(banner.as_ref().map(serde_json::to_string).transpose()?)
    .bind(serde_json::to_string(&descriptions)?)
    .bind(blog.enabled)
    .bind(&source_hash)
    .bind(now_rfc3339())
    .execute(&mut **tx)
    .await?;
    Ok(true)
}

/// Checked on every sync, not only when `site.yaml` changes: removing the
/// default theme's file must fail as well.
async fn check_default_theme(tx: &mut Tx<'_>) -> Result<(), Error> {
    let (theme, exists): (String, bool) = sqlx::query_as(
        "SELECT theme, EXISTS (SELECT 1 FROM themes WHERE slug = site.theme)
         FROM site WHERE id = 1",
    )
    .fetch_one(&mut **tx)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(Error::file(
            manifest::path(Kind::Site, None),
            format!("theme {theme} has no themes/{theme}.yaml"),
        ))
    }
}

async fn sync_themes(
    tx: &mut Tx<'_>,
    content_dir: &Path,
    report: &mut SyncReport,
) -> Result<HashSet<String>, Error> {
    let mut seen = HashSet::new();
    for rel in listed(content_dir, Kind::Theme)? {
        let slug = claim(&rel, &mut seen)?;
        let bytes = read(content_dir, &rel)?;
        let source_hash = hash(&bytes);
        let existing: Option<String> =
            sqlx::query_scalar("SELECT source_hash FROM themes WHERE slug = ?1")
                .bind(&slug)
                .fetch_optional(&mut **tx)
                .await?;
        if existing.as_deref() == Some(source_hash.as_str()) {
            report.themes_skipped += 1;
            continue;
        }
        let Manifest::Theme {
            spec:
                ThemeSpec {
                    title,
                    dark,
                    colors,
                },
            ..
        } = manifest::parse(&rel, &bytes)?
        else {
            return Err(Error::file(&rel, "not a theme"));
        };
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
        .bind(&slug)
        .bind(&title)
        .bind(dark)
        .bind(serde_json::to_string(&colors)?)
        .bind(&source_hash)
        .bind(now_rfc3339())
        .execute(&mut **tx)
        .await?;
        report.themes_upserted += 1;
    }
    Ok(seen)
}

async fn sync_profile(
    tx: &mut Tx<'_>,
    content_dir: &Path,
    markdown: &Markdown,
) -> Result<bool, Error> {
    let rel = PathBuf::from(manifest::path(Kind::Profile, None));
    let bytes = read(content_dir, &rel)?;
    let source_hash = hash(&bytes);
    let existing: Option<String> =
        sqlx::query_scalar("SELECT source_hash FROM profile WHERE id = 1")
            .fetch_optional(&mut **tx)
            .await?;
    if existing.as_deref() == Some(source_hash.as_str()) {
        return Ok(false);
    }

    let Manifest::Profile(spec) = manifest::parse(&rel, &bytes)? else {
        return Err(Error::file(&rel, "not a profile"));
    };
    let ProfileSpec {
        name,
        headline,
        bio,
        email,
        links,
        body,
    } = spec;
    let links = serde_json::to_string(&links)?;
    let about_html = markdown.html(&body);
    let about_text = Markdown::text(&body);

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
    .bind(&name)
    .bind(&headline)
    .bind(&bio)
    .bind(&body)
    .bind(&about_html)
    .bind(&about_text)
    .bind(&email)
    .bind(&links)
    .bind(&source_hash)
    .bind(now_rfc3339())
    .execute(&mut **tx)
    .await?;

    Ok(true)
}

async fn sync_projects(
    tx: &mut Tx<'_>,
    content_dir: &Path,
    markdown: &Markdown,
    report: &mut SyncReport,
) -> Result<HashSet<String>, Error> {
    let mut seen = HashSet::new();
    for rel in listed(content_dir, Kind::Project)? {
        if upsert_project(tx, content_dir, &rel, markdown, &mut seen).await? {
            report.projects_upserted += 1;
        } else {
            report.projects_skipped += 1;
        }
    }

    Ok(seen)
}

async fn upsert_project(
    tx: &mut Tx<'_>,
    content_dir: &Path,
    rel: &Path,
    markdown: &Markdown,
    seen: &mut HashSet<String>,
) -> Result<bool, Error> {
    let slug = claim(rel, seen)?;

    let bytes = read(content_dir, rel)?;
    let source_hash = hash(&bytes);
    let existing: Option<String> =
        sqlx::query_scalar("SELECT source_hash FROM projects WHERE slug = ?1")
            .bind(&slug)
            .fetch_optional(&mut **tx)
            .await?;
    if existing.as_deref() == Some(source_hash.as_str()) {
        return Ok(false);
    }

    let Manifest::Project { spec, .. } = manifest::parse(rel, &bytes)? else {
        return Err(Error::file(rel, "not a project"));
    };
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

    let now = now_rfc3339();
    let tags = serde_json::to_string(&tags)?;
    let links = serde_json::to_string(&links)?;
    let body_html = markdown.html(&body);
    let body_text = Markdown::text(&body);

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
    .bind(&slug)
    .bind(&title)
    .bind(&tagline)
    .bind(status.as_str())
    .bind(draft)
    .bind(highlight)
    .bind(&tags)
    .bind(&links)
    .bind(&github)
    .bind(&body)
    .bind(&body_html)
    .bind(&body_text)
    .bind(&source_hash)
    .bind(&now)
    .execute(&mut **tx)
    .await?;

    sqlx::query("DELETE FROM project_assets WHERE project_slug = ?1")
        .bind(&slug)
        .execute(&mut **tx)
        .await?;

    for asset in &assets {
        sqlx::query(
            "INSERT INTO project_assets (project_slug, path, caption, width, height, hash, derivatives)
             VALUES (?1, ?2, ?3, 0, 0, '', '[]')",
        )
        .bind(&slug)
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
