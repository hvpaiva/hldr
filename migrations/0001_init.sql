CREATE TABLE projects (
    slug TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    tagline TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'wip', 'archived')),
    highlight INTEGER,
    tags TEXT NOT NULL,
    links TEXT NOT NULL,
    github_repo TEXT,
    body_source TEXT NOT NULL,
    body_html TEXT NOT NULL,
    body_text TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    last_applied TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE project_assets (
    id INTEGER PRIMARY KEY,
    project_slug TEXT NOT NULL REFERENCES projects (slug) ON DELETE CASCADE,
    path TEXT NOT NULL,
    caption TEXT,
    width INTEGER NOT NULL DEFAULT 0,
    height INTEGER NOT NULL DEFAULT 0,
    hash TEXT NOT NULL DEFAULT '',
    derivatives TEXT NOT NULL DEFAULT '[]',
    UNIQUE (project_slug, path)
);

CREATE TABLE profile (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    name TEXT NOT NULL,
    headline TEXT NOT NULL,
    bio TEXT NOT NULL,
    about_source TEXT NOT NULL DEFAULT '',
    about_html TEXT NOT NULL DEFAULT '',
    about_text TEXT NOT NULL DEFAULT '',
    email TEXT,
    links TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

INSERT INTO config (key, value, updated_at) VALUES (
    'blog.enabled',
    'false',
    '1970-01-01T00:00:00Z'
);
