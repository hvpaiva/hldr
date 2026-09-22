-- Page views counted without tracking: no cookie, no script, no IP kept.
--
-- A visitor is a hash of the day's salt, the client's address and its user
-- agent, kept only while its UTC day is open. When the day closes, its
-- visitors are counted into the rows below and its hashes and salt are
-- deleted, so nobody can be followed from one day to the next. Closed days
-- are kept forever: a few rows each.
--
-- Every write is additive or idempotent, so the two servers of a deploy can
-- flush into the same database.

-- One row per UTC day that saw a page or a bot.
CREATE TABLE daily (
    day TEXT PRIMARY KEY,
    -- Pages served, the 404 bucket aside.
    views INTEGER NOT NULL DEFAULT 0,
    -- Distinct visitors of the day, written when it closes.
    visitors INTEGER NOT NULL DEFAULT 0,
    -- Requests from user agents that declare themselves bots.
    bots INTEGER NOT NULL DEFAULT 0,
    -- Once set, the day takes no more visitor hashes.
    closed INTEGER NOT NULL DEFAULT 0 CHECK (closed IN (0, 1))
);

-- Views per route; every 404 shares the route `404`.
CREATE TABLE page_views (
    day TEXT NOT NULL,
    route TEXT NOT NULL,
    views INTEGER NOT NULL DEFAULT 0,
    -- Written when the day closes.
    visitors INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, route)
) WITHOUT ROWID;

-- Views per referring host, lowercased, the site itself aside.
CREATE TABLE referrers (
    day TEXT NOT NULL,
    host TEXT NOT NULL,
    views INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, host)
) WITHOUT ROWID;

-- Created by whichever server needs it first, so every server hashes a
-- visitor alike; deleted when the day closes.
CREATE TABLE metrics_salt (
    day TEXT PRIMARY KEY,
    salt BLOB NOT NULL CHECK (length(salt) = 32)
);

-- sha256(salt, address, user agent) per route, while the day is open.
CREATE TABLE visitor_hashes (
    day TEXT NOT NULL,
    route TEXT NOT NULL,
    hash BLOB NOT NULL CHECK (length(hash) = 32),
    PRIMARY KEY (day, route, hash)
) WITHOUT ROWID;
