-- What happened to the server: boots, stops, syncs and their failures.
--
-- Unlike content, events are not rebuilt from git: they are kept 30 days
-- and lost with the database, as Kubernetes loses its events.
CREATE TABLE events (
    id INTEGER PRIMARY KEY,
    -- Bumped on every insert and every update, from `event_seq`, so a watch
    -- that asks for what came after the last seq it saw sees both.
    seq INTEGER NOT NULL UNIQUE,
    type TEXT NOT NULL CHECK (type IN ('Normal', 'Warning')),
    reason TEXT NOT NULL,
    message TEXT NOT NULL,
    revision TEXT,
    count INTEGER NOT NULL DEFAULT 1 CHECK (count > 0),
    first_at TEXT NOT NULL,
    last_at TEXT NOT NULL
);

CREATE INDEX events_last_at ON events (last_at);

-- A counter rather than max(seq): pruning must never hand a seq out again.
CREATE TABLE event_seq (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    seq INTEGER NOT NULL
);

INSERT INTO event_seq (id, seq) VALUES (1, 0);
