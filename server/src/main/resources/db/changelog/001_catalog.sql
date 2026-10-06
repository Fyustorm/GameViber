--liquibase formatted sql

-- Moments are milliseconds since the epoch (`EpochMillis`).

--changeset gameviber:001-game
CREATE TABLE game (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    -- The name lower-cased, letters and digits only: "ELDEN RING" and "Elden Ring" are one game.
    slug TEXT NOT NULL UNIQUE,
    steam_app_id INTEGER UNIQUE,
    created_at INTEGER NOT NULL
);

--changeset gameviber:001-author
CREATE TABLE author (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    pseudo TEXT NOT NULL,
    -- The pseudo lower-cased: "Fyu" and "fyu" are one author.
    pseudo_key TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    blocked INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

--changeset gameviber:001-mode
CREATE TABLE mode (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- What links and the app name the mode by (the row id says how many there are).
    public_id TEXT NOT NULL UNIQUE,
    game_id INTEGER NOT NULL REFERENCES game (id),
    author_id INTEGER NOT NULL REFERENCES author (id),
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    -- 'private': only through its share code; 'public': listed.
    visibility TEXT NOT NULL CHECK (visibility IN ('private', 'public')),
    share_code TEXT UNIQUE,
    downloads INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    -- Withdrawn by its author or an administrator: neither listed nor downloadable.
    withdrawn_at INTEGER,
    withdrawn_reason TEXT
);
CREATE INDEX mode_game ON mode (game_id);
CREATE INDEX mode_author ON mode (author_id);

--changeset gameviber:001-mode-version
CREATE TABLE mode_version (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mode_id INTEGER NOT NULL REFERENCES mode (id),
    -- 1, 2, 3... in the order they were published; its package is packages/<public id>/<number>.gameviber.
    number INTEGER NOT NULL,
    changelog TEXT NOT NULL DEFAULT '',
    -- The mode API version its script declares (`api = 1`).
    api INTEGER NOT NULL,
    size INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (mode_id, number)
);

--changeset gameviber:001-download
CREATE TABLE download (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mode_id INTEGER NOT NULL REFERENCES mode (id),
    version_number INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX download_mode ON download (mode_id);

--changeset gameviber:001-report
CREATE TABLE report (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mode_id INTEGER NOT NULL REFERENCES mode (id),
    -- 'broken' (a game update broke it), 'content', 'other'.
    reason TEXT NOT NULL CHECK (reason IN ('broken', 'content', 'other')),
    details TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    resolved_at INTEGER
);
CREATE INDEX report_mode ON report (mode_id);
