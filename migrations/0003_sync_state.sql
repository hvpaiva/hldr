-- Which content revision the database materializes, and how the last
-- attempt to refresh it went.
CREATE TABLE sync_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision TEXT,
    synced_at TEXT,
    last_attempt_at TEXT NOT NULL,
    last_error TEXT
);
