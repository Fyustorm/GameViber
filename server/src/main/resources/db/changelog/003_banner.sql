--liquibase formatted sql

-- A game's banner: the header image of its Steam store page, as Steam's store
-- API gives it (`SteamBanners`): recent games keep it under a path with a hash.
-- Null while not asked yet, or Steam gave none: the path without a hash then.

--changeset gameviber:003-banner
ALTER TABLE game ADD COLUMN banner_url TEXT;
-- When Steam was last asked, for this Steam app id.
ALTER TABLE game ADD COLUMN banner_checked_at INTEGER;
