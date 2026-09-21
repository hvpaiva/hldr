-- Git holds the desired state; writes replace files there, so the
-- database no longer tracks what a client last applied.
ALTER TABLE projects DROP COLUMN last_applied;

ALTER TABLE projects ADD COLUMN draft INTEGER NOT NULL DEFAULT 0 CHECK (draft IN (0, 1));
