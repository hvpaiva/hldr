-- Closed days the metrics repository holds, as the server last saw it:
-- written after an upload, or for each day a restore brought back. A closed
-- day missing here is uploaded next.
CREATE TABLE metrics_backup (
    day TEXT PRIMARY KEY,
    uploaded_at TEXT NOT NULL,
    -- Git blob id of the day's file in the repository; null for a day a
    -- restore brought back, since the tarball carries no ids.
    sha TEXT
);
