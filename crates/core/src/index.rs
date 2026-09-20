use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::Error;
use crate::markdown::Markdown;
use crate::types::{AssetSpec, ProfileLinks, ProjectLinks, ProjectStatus};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub profile_updated: bool,
    pub projects_upserted: u32,
    pub projects_skipped: u32,
    pub projects_deleted: u32,
}

#[derive(Debug, Deserialize)]
struct ProfileFile {
    name: String,
    headline: String,
    bio: String,
    #[serde(default)]
    about: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    links: ProfileLinks,
}

#[derive(Debug, Deserialize)]
struct ProjectFrontmatter {
    title: String,
    tagline: String,
    status: ProjectStatus,
    #[serde(default)]
    highlight: Option<i64>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    links: ProjectLinks,
    #[serde(default)]
    github: Option<String>,
    #[serde(default)]
    assets: Vec<AssetSpec>,
}

pub async fn sync(pool: &SqlitePool, content_dir: &Path) -> Result<SyncReport, Error> {
    if !content_dir.is_dir() {
        return Err(Error::file(content_dir, "content directory does not exist"));
    }

    let markdown = Markdown::new();
    let mut tx = pool.begin().await?;
    let mut report = SyncReport::default();

    sync_profile(&mut tx, content_dir, &markdown, &mut report).await?;
    let seen = sync_projects(&mut tx, content_dir, &markdown, &mut report).await?;
    report.projects_deleted = delete_missing(&mut tx, &seen).await?;

    tx.commit().await?;
    Ok(report)
}

async fn sync_profile(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    content_dir: &Path,
    markdown: &Markdown,
    report: &mut SyncReport,
) -> Result<(), Error> {
    let path = content_dir.join("profile.yaml");
    let bytes = fs::read(&path).map_err(|err| Error::file(&path, err.to_string()))?;
    let source_hash = hash(&bytes);
    let existing: Option<String> =
        sqlx::query_scalar("SELECT source_hash FROM profile WHERE id = 1")
            .fetch_optional(&mut **tx)
            .await?;
    if existing.as_deref() == Some(source_hash.as_str()) {
        return Ok(());
    }

    let parsed: ProfileFile = parse_yaml(&path, &bytes)?;
    let now = now_rfc3339();
    let links = serde_json::to_string(&parsed.links)?;
    let about_html = markdown.html(&parsed.about);
    let about_text = Markdown::text(&parsed.about);

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
    .bind(&parsed.name)
    .bind(&parsed.headline)
    .bind(&parsed.bio)
    .bind(&parsed.about)
    .bind(&about_html)
    .bind(&about_text)
    .bind(&parsed.email)
    .bind(&links)
    .bind(&source_hash)
    .bind(&now)
    .execute(&mut **tx)
    .await?;

    report.profile_updated = true;
    Ok(())
}

async fn sync_projects(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    content_dir: &Path,
    markdown: &Markdown,
    report: &mut SyncReport,
) -> Result<HashSet<String>, Error> {
    let mut seen = HashSet::new();
    let projects_dir = content_dir.join("projects");
    if !projects_dir.exists() {
        return Ok(seen);
    }

    let mut entries: Vec<PathBuf> = fs::read_dir(&projects_dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    entries.sort();

    for path in entries {
        let slug = slug_from_path(&path)?;
        if !seen.insert(slug.clone()) {
            return Err(Error::file(&path, "duplicate slug"));
        }
        if upsert_project(tx, &path, &slug, markdown).await? {
            report.projects_upserted += 1;
        } else {
            report.projects_skipped += 1;
        }
    }

    Ok(seen)
}

async fn upsert_project(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    path: &Path,
    slug: &str,
    markdown: &Markdown,
) -> Result<bool, Error> {
    let bytes = fs::read(path)?;
    let source_hash = hash(&bytes);
    let existing: Option<String> =
        sqlx::query_scalar("SELECT source_hash FROM projects WHERE slug = ?1")
            .bind(slug)
            .fetch_optional(&mut **tx)
            .await?;
    if existing.as_deref() == Some(source_hash.as_str()) {
        return Ok(false);
    }

    let text =
        std::str::from_utf8(&bytes).map_err(|_| Error::file(path, "file is not valid UTF-8"))?;
    let (frontmatter, body) = split_frontmatter(text)?;
    let meta: ProjectFrontmatter =
        serde_saphyr::from_str(frontmatter).map_err(|err| Error::file(path, err.to_string()))?;

    let now = now_rfc3339();
    let tags = serde_json::to_string(&meta.tags)?;
    let links = serde_json::to_string(&meta.links)?;
    let body_html = markdown.html(body);
    let body_text = Markdown::text(body);

    sqlx::query(
        "INSERT INTO projects (
            slug, title, tagline, status, highlight, tags, links, github_repo,
            body_source, body_html, body_text, source_hash, last_applied,
            created_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, ?13, ?13
        )
        ON CONFLICT(slug) DO UPDATE SET
            title = excluded.title,
            tagline = excluded.tagline,
            status = excluded.status,
            highlight = excluded.highlight,
            tags = excluded.tags,
            links = excluded.links,
            github_repo = excluded.github_repo,
            body_source = excluded.body_source,
            body_html = excluded.body_html,
            body_text = excluded.body_text,
            source_hash = excluded.source_hash,
            created_at = projects.created_at,
            last_applied = projects.last_applied,
            updated_at = excluded.updated_at",
    )
    .bind(slug)
    .bind(&meta.title)
    .bind(&meta.tagline)
    .bind(meta.status.as_str())
    .bind(meta.highlight)
    .bind(&tags)
    .bind(&links)
    .bind(&meta.github)
    .bind(body)
    .bind(&body_html)
    .bind(&body_text)
    .bind(&source_hash)
    .bind(&now)
    .execute(&mut **tx)
    .await?;

    sqlx::query("DELETE FROM project_assets WHERE project_slug = ?1")
        .bind(slug)
        .execute(&mut **tx)
        .await?;

    for asset in &meta.assets {
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

async fn delete_missing(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    seen: &HashSet<String>,
) -> Result<u32, Error> {
    let slugs: Vec<String> = sqlx::query_scalar("SELECT slug FROM projects")
        .fetch_all(&mut **tx)
        .await?;
    let mut deleted = 0;
    for slug in slugs {
        if !seen.contains(&slug) {
            sqlx::query("DELETE FROM projects WHERE slug = ?1")
                .bind(&slug)
                .execute(&mut **tx)
                .await?;
            deleted += 1;
        }
    }
    Ok(deleted)
}

fn slug_from_path(path: &Path) -> Result<String, Error> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::file(path, "invalid file name"))?;
    if stem.is_empty()
        || !stem
            .chars()
            .enumerate()
            .all(|(i, c)| c.is_ascii_lowercase() || c.is_ascii_digit() || (i > 0 && c == '-'))
    {
        return Err(Error::file(path, "slug must match [a-z0-9][a-z0-9-]*"));
    }
    Ok(stem.to_owned())
}

fn split_frontmatter(source: &str) -> Result<(&str, &str), Error> {
    let rest = source
        .strip_prefix("---\n")
        .or_else(|| source.strip_prefix("---\r\n"))
        .ok_or(Error::Frontmatter("missing opening ---"))?;
    rest.split_once("\n---\n")
        .or_else(|| rest.split_once("\r\n---\r\n"))
        .or_else(|| rest.split_once("\n---\r\n"))
        .or_else(|| rest.split_once("\r\n---\n"))
        .ok_or(Error::Frontmatter("missing closing ---"))
}

fn parse_yaml<T: serde::de::DeserializeOwned>(path: &Path, bytes: &[u8]) -> Result<T, Error> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| Error::file(path, "file is not valid UTF-8"))?;
    serde_saphyr::from_str(text).map_err(|err| Error::file(path, err.to_string()))
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

    const PROFILE: &str = "\
name: Highlander Paiva
headline: senior platform engineer
bio: writes software.
email: contact@hvpaiva.dev
links:
  github: https://github.com/hvpaiva
";

    const PROJECT: &str = "\
---
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
        fs::write(dir.join("profile.yaml"), PROFILE).unwrap();
        fs::create_dir(dir.join("projects")).unwrap();
        fs::write(dir.join("projects/atlas.md"), PROJECT).unwrap();
        Db::open(dir.join("hldr.db")).await.unwrap()
    }

    #[tokio::test]
    async fn indexes_profile_and_project() {
        let dir = tempfile::tempdir().unwrap();
        let db = setup(dir.path()).await;
        let report = sync(db.pool(), dir.path()).await.unwrap();
        assert!(report.profile_updated);
        assert_eq!(report.projects_upserted, 1);
        assert_eq!(report.projects_skipped, 0);

        let title: String = sqlx::query_scalar("SELECT title FROM projects WHERE slug = 'atlas'")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(title, "Atlas");

        let html: String =
            sqlx::query_scalar("SELECT body_html FROM projects WHERE slug = 'atlas'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert!(html.contains("<strong>decisions</strong>"), "{html}");

        let caption: String =
            sqlx::query_scalar("SELECT caption FROM project_assets WHERE project_slug = 'atlas'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(caption, "pipeline");

        let enabled: String =
            sqlx::query_scalar("SELECT value FROM config WHERE key = 'blog.enabled'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(enabled, "false");
    }

    #[tokio::test]
    async fn second_pass_skips_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let db = setup(dir.path()).await;
        sync(db.pool(), dir.path()).await.unwrap();
        let report = sync(db.pool(), dir.path()).await.unwrap();
        assert!(!report.profile_updated);
        assert_eq!(report.projects_upserted, 0);
        assert_eq!(report.projects_skipped, 1);
        assert_eq!(report.projects_deleted, 0);
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
    async fn preserves_created_at_and_last_applied() {
        let dir = tempfile::tempdir().unwrap();
        let db = setup(dir.path()).await;
        sync(db.pool(), dir.path()).await.unwrap();
        sqlx::query(
            "UPDATE projects SET last_applied = '{\"k\":1}', created_at = '2020-01-01T00:00:00Z'",
        )
        .execute(db.pool())
        .await
        .unwrap();
        fs::write(
            dir.path().join("projects/atlas.md"),
            PROJECT.replace("CubeSat ADCS in Rust", "updated tagline"),
        )
        .unwrap();
        sync(db.pool(), dir.path()).await.unwrap();
        let (created_at, last_applied, tagline): (String, Option<String>, String) = sqlx::query_as(
            "SELECT created_at, last_applied, tagline FROM projects WHERE slug = 'atlas'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(created_at, "2020-01-01T00:00:00Z");
        assert_eq!(last_applied.as_deref(), Some("{\"k\":1}"));
        assert_eq!(tagline, "updated tagline");
    }

    #[test]
    fn split_frontmatter_roundtrip() {
        let (fm, body) = split_frontmatter(PROJECT).unwrap();
        assert!(fm.contains("title: Atlas"));
        assert!(body.contains("Context"));
    }
}
