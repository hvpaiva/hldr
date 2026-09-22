-- Pages, navigation and page types move from the binary into content.
--
-- Only additions: during a deploy the previous release keeps serving while
-- this one boots, reading `projects`, `project_assets`, `profile.about_*`
-- and the `banner`, `descriptions` and `blog_enabled` columns of `site`.
-- This release never writes those, so they keep what that release needs,
-- and a later migration drops them.
CREATE TABLE nav (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    spec TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE page_kinds (
    name TEXT PRIMARY KEY,
    spec TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE collections (
    name TEXT PRIMARY KEY,
    spec TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Every page of every type. `content` is the markdown, from the file the
-- manifest names or inline; `source_hash` covers both.
CREATE TABLE pages (
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    collection TEXT,
    metadata TEXT NOT NULL,
    spec TEXT NOT NULL,
    content TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (kind, name)
);

ALTER TABLE site ADD COLUMN values_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE themes ADD COLUMN sort_order INTEGER;
ALTER TABLE themes ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1));
