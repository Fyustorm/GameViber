--liquibase formatted sql

-- What players who share their stats send: how long they play the modes they
-- installed, and how they rate them. An installation is a random id the app
-- made, stored hashed with a salt of the server's (`Stats`), and linked to no
-- author: nothing tells who plays.

--changeset gameviber:002-play
CREATE TABLE play (
    mode_id INTEGER NOT NULL REFERENCES mode (id),
    installation TEXT NOT NULL,
    -- Days since the epoch: plays add up per day.
    day INTEGER NOT NULL,
    seconds INTEGER NOT NULL,
    sessions INTEGER NOT NULL,
    PRIMARY KEY (mode_id, installation, day)
);
CREATE INDEX play_day ON play (day);

--changeset gameviber:002-vote
CREATE TABLE vote (
    mode_id INTEGER NOT NULL REFERENCES mode (id),
    installation TEXT NOT NULL,
    -- 1: liked, -1: not.
    value INTEGER NOT NULL CHECK (value IN (1, -1)),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (mode_id, installation)
);
