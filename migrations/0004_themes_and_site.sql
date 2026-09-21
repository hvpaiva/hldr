-- Color schemes and site identity move from the binary into content.
--
-- `config` stays for now: during a deploy the previous release still reads
-- it while this one boots. A later migration drops it.
CREATE TABLE themes (
    slug TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    dark INTEGER NOT NULL CHECK (dark IN (0, 1)),
    colors TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE site (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    title TEXT NOT NULL,
    theme TEXT NOT NULL,
    banner TEXT,
    descriptions TEXT NOT NULL,
    blog_enabled INTEGER NOT NULL CHECK (blog_enabled IN (0, 1)),
    source_hash TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
