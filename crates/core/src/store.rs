use serde::Serialize;
use sqlx::FromRow;

use crate::types::{ProfileLinks, ProjectLinks, ProjectStatus};
use crate::{Db, Error};

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
    /// Markdown body exactly as written below the frontmatter.
    pub body_source: String,
    pub body_html: String,
    pub body_text: String,
    pub created_at: String,
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
        let value: Option<String> =
            sqlx::query_scalar("SELECT value FROM config WHERE key = 'blog.enabled'")
                .fetch_optional(self.pool())
                .await?;
        Ok(matches!(value.as_deref(), Some("true") | Some("1")))
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

    pub async fn projects(&self) -> Result<Vec<ProjectSummary>, Error> {
        let rows: Vec<ProjectRow> = sqlx::query_as(
            "SELECT slug, title, tagline, status, highlight, tags, links, github_repo,
                    '' AS body_source, '' AS body_html, '' AS body_text, created_at, updated_at
             FROM projects
             ORDER BY CASE WHEN highlight IS NULL THEN 1 ELSE 0 END,
                      highlight ASC, updated_at DESC",
        )
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

    pub async fn project(&self, slug: &str) -> Result<Option<Project>, Error> {
        let row: Option<ProjectRow> = sqlx::query_as(
            "SELECT slug, title, tagline, status, highlight, tags, links, github_repo,
                    body_source, body_html, body_text, created_at, updated_at
             FROM projects WHERE slug = ?1",
        )
        .bind(slug)
        .fetch_optional(self.pool())
        .await?;
        row.map(project_from_row).transpose()
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

fn project_from_row(row: ProjectRow) -> Result<Project, Error> {
    Ok(Project {
        slug: row.slug,
        title: row.title,
        tagline: row.tagline,
        status: ProjectStatus::parse(&row.status)?,
        highlight: row.highlight,
        tags: serde_json::from_str(&row.tags)?,
        links: serde_json::from_str(&row.links)?,
        github_repo: row.github_repo,
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

    const PROFILE: &str = "\
name: Highlander Paiva
headline: senior platform engineer
bio: writes software.
email: contact@hvpaiva.dev
links:
  github: https://github.com/hvpaiva
about: |
  design to platform.
";

    const PROJECT: &str = "\
---
title: Atlas
tagline: CubeSat ADCS in Rust
status: active
highlight: 1
tags: [rust]
links:
  repo: https://github.com/hvpaiva/atlas
---

Body.
";

    async fn seeded() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("profile.yaml"), PROFILE).unwrap();
        fs::create_dir(dir.path().join("projects")).unwrap();
        fs::write(dir.path().join("projects/atlas.md"), PROJECT).unwrap();
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
        assert!(!db.blog_enabled().await.unwrap());
        assert_eq!(db.project_count().await.unwrap(), 1);
        db.ping().await.unwrap();
    }
}
