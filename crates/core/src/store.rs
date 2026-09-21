use std::collections::HashMap;

use serde::Serialize;
use sqlx::FromRow;

use crate::types::{AssetSpec, ProfileLinks, ProjectLinks, ProjectStatus};
use crate::{Db, Error};

// Highlights first, in their order, then the most recently changed.
macro_rules! ordered {
    ($select:literal) => {
        concat!(
            $select,
            " ORDER BY CASE WHEN highlight IS NULL THEN 1 ELSE 0 END,",
            " highlight ASC, updated_at DESC"
        )
    };
}

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub name: String,
    pub headline: String,
    pub bio: String,
    pub about_source: String,
    pub about_html: String,
    pub about_text: String,
    pub email: Option<String>,
    pub links: ProfileLinks,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectSummary {
    pub slug: String,
    pub title: String,
    pub tagline: String,
    pub status: ProjectStatus,
    pub highlight: Option<i64>,
    pub tags: Vec<String>,
    pub links: ProjectLinks,
    pub github_repo: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Project {
    pub slug: String,
    pub title: String,
    pub tagline: String,
    pub status: ProjectStatus,
    pub highlight: Option<i64>,
    pub tags: Vec<String>,
    pub links: ProjectLinks,
    pub github_repo: Option<String>,
    pub draft: bool,
    pub assets: Vec<AssetSpec>,
    /// Markdown body exactly as written below the frontmatter.
    pub body_source: String,
    pub body_html: String,
    pub body_text: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SiteConfig {
    pub blog_enabled: bool,
    pub updated_at: String,
}

#[derive(FromRow)]
struct ProfileRow {
    name: String,
    headline: String,
    bio: String,
    about_source: String,
    about_html: String,
    about_text: String,
    email: Option<String>,
    links: String,
    updated_at: String,
}

#[derive(FromRow)]
struct ProjectRow {
    slug: String,
    title: String,
    tagline: String,
    status: String,
    highlight: Option<i64>,
    tags: String,
    links: String,
    github_repo: Option<String>,
    draft: bool,
    body_source: String,
    body_html: String,
    body_text: String,
    created_at: String,
    updated_at: String,
}

impl Db {
    pub async fn ping(&self) -> Result<(), Error> {
        sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(self.pool())
            .await?;
        Ok(())
    }

    pub async fn blog_enabled(&self) -> Result<bool, Error> {
        Ok(self.site_config().await?.blog_enabled)
    }

    pub async fn site_config(&self) -> Result<SiteConfig, Error> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT value, updated_at FROM config WHERE key = 'blog.enabled'")
                .fetch_optional(self.pool())
                .await?;
        let (value, updated_at) = row.ok_or(Error::Invariant("config blog.enabled missing"))?;
        Ok(SiteConfig {
            blog_enabled: matches!(value.as_str(), "true" | "1"),
            updated_at,
        })
    }

    pub async fn profile(&self) -> Result<Profile, Error> {
        let row: ProfileRow = sqlx::query_as(
            "SELECT name, headline, bio, about_source, about_html, about_text,
                    email, links, updated_at
             FROM profile WHERE id = 1",
        )
        .fetch_optional(self.pool())
        .await?
        .ok_or(Error::Invariant("profile missing"))?;
        Ok(Profile {
            name: row.name,
            headline: row.headline,
            bio: row.bio,
            about_source: row.about_source,
            about_html: row.about_html,
            about_text: row.about_text,
            email: row.email,
            links: serde_json::from_str(&row.links)?,
            updated_at: row.updated_at,
        })
    }

    /// Published projects, without bodies.
    pub async fn projects(&self) -> Result<Vec<ProjectSummary>, Error> {
        let rows: Vec<ProjectRow> = sqlx::query_as(ordered!(
            "SELECT slug, title, tagline, status, highlight, tags, links, github_repo, draft,
                    '' AS body_source, '' AS body_html, '' AS body_text, created_at, updated_at
             FROM projects WHERE draft = 0"
        ))
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(summary_from_row).collect()
    }

    pub async fn highlighted_projects(&self) -> Result<Vec<ProjectSummary>, Error> {
        let all = self.projects().await?;
        let highlighted: Vec<ProjectSummary> = all
            .iter()
            .filter(|project| project.highlight.is_some())
            .cloned()
            .collect();
        if highlighted.is_empty() {
            Ok(all)
        } else {
            Ok(highlighted)
        }
    }

    /// A published project.
    pub async fn project(&self, slug: &str) -> Result<Option<Project>, Error> {
        Ok(self
            .any_project(slug)
            .await?
            .filter(|project| !project.draft))
    }

    /// A project, drafts included.
    pub async fn any_project(&self, slug: &str) -> Result<Option<Project>, Error> {
        let row: Option<ProjectRow> = sqlx::query_as(
            "SELECT slug, title, tagline, status, highlight, tags, links, github_repo, draft,
                    body_source, body_html, body_text, created_at, updated_at
             FROM projects WHERE slug = ?1",
        )
        .bind(slug)
        .fetch_optional(self.pool())
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let mut assets = self.assets(Some(slug)).await?;
        let assets = assets.remove(slug).unwrap_or_default();
        project_from_row(row, assets).map(Some)
    }

    /// Every project with its body, drafts included.
    pub async fn all_projects(&self) -> Result<Vec<Project>, Error> {
        let rows: Vec<ProjectRow> = sqlx::query_as(ordered!(
            "SELECT slug, title, tagline, status, highlight, tags, links, github_repo, draft,
                    body_source, body_html, body_text, created_at, updated_at
             FROM projects"
        ))
        .fetch_all(self.pool())
        .await?;
        let mut assets = self.assets(None).await?;
        rows.into_iter()
            .map(|row| {
                let own = assets.remove(&row.slug).unwrap_or_default();
                project_from_row(row, own)
            })
            .collect()
    }

    async fn assets(&self, slug: Option<&str>) -> Result<HashMap<String, Vec<AssetSpec>>, Error> {
        let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT project_slug, path, caption FROM project_assets
             WHERE ?1 IS NULL OR project_slug = ?1
             ORDER BY id",
        )
        .bind(slug)
        .fetch_all(self.pool())
        .await?;
        let mut out: HashMap<String, Vec<AssetSpec>> = HashMap::new();
        for (project, path, caption) in rows {
            out.entry(project)
                .or_default()
                .push(AssetSpec { path, caption });
        }
        Ok(out)
    }

    pub async fn project_count(&self) -> Result<i64, Error> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM projects")
            .fetch_one(self.pool())
            .await?)
    }
}

fn summary_from_row(row: ProjectRow) -> Result<ProjectSummary, Error> {
    Ok(ProjectSummary {
        slug: row.slug,
        title: row.title,
        tagline: row.tagline,
        status: ProjectStatus::parse(&row.status)?,
        highlight: row.highlight,
        tags: serde_json::from_str(&row.tags)?,
        links: serde_json::from_str(&row.links)?,
        github_repo: row.github_repo,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn project_from_row(row: ProjectRow, assets: Vec<AssetSpec>) -> Result<Project, Error> {
    Ok(Project {
        slug: row.slug,
        title: row.title,
        tagline: row.tagline,
        status: ProjectStatus::parse(&row.status)?,
        highlight: row.highlight,
        tags: serde_json::from_str(&row.tags)?,
        links: serde_json::from_str(&row.links)?,
        github_repo: row.github_repo,
        draft: row.draft,
        assets,
        body_source: row.body_source,
        body_html: row.body_html,
        body_text: row.body_text,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index;
    use std::fs;

    const SITE: &str = "kind: Site\nblog:\n  enabled: false\n";

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
design to platform.
";

    const PROJECT: &str = "\
---
kind: Project
title: Atlas
tagline: CubeSat ADCS in Rust
status: active
highlight: 1
tags: [rust]
links:
  repo: https://github.com/hvpaiva/atlas
assets:
  - path: atlas/overview.png
---

Body.
";

    async fn seeded() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("site.yaml"), SITE).unwrap();
        fs::write(dir.path().join("profile.md"), PROFILE).unwrap();
        fs::create_dir(dir.path().join("projects")).unwrap();
        fs::write(dir.path().join("projects/atlas.md"), PROJECT).unwrap();
        let draft = PROJECT
            .replace("title: Atlas", "title: Hidden\ndraft: true")
            .replace("highlight: 1\n", "")
            .replace("assets:\n  - path: atlas/overview.png\n", "");
        fs::write(dir.path().join("projects/hidden.md"), draft).unwrap();
        let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
        index::sync(db.pool(), dir.path()).await.unwrap();
        (dir, db)
    }

    #[tokio::test]
    async fn reads_profile_and_project() {
        let (_dir, db) = seeded().await;
        let profile = db.profile().await.unwrap();
        assert_eq!(profile.name, "Highlander Paiva");
        assert!(profile.about_html.contains("design"));

        let listed = db.projects().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].slug, "atlas");

        let project = db.project("atlas").await.unwrap().unwrap();
        assert!(project.body_html.contains("Body"));
        assert_eq!(project.body_source.trim(), "Body.");
        assert_eq!(project.assets[0].path, "atlas/overview.png");
        assert!(!db.blog_enabled().await.unwrap());
        assert_eq!(db.project_count().await.unwrap(), 2);
        db.ping().await.unwrap();
    }

    #[tokio::test]
    async fn drafts_stay_out_of_public_reads() {
        let (_dir, db) = seeded().await;
        assert!(db.project("hidden").await.unwrap().is_none());
        assert!(
            db.projects()
                .await
                .unwrap()
                .iter()
                .all(|p| p.slug != "hidden")
        );

        let draft = db.any_project("hidden").await.unwrap().unwrap();
        assert!(draft.draft);
        let all = db.all_projects().await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].slug, "atlas");
        assert_eq!(all[0].assets.len(), 1);
        assert_eq!(all[1].slug, "hidden");
        assert!(all[1].assets.is_empty());
    }
}
