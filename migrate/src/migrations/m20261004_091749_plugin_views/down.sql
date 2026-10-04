ALTER TABLE message DROP COLUMN card;
DROP TABLE plugin_capability;
DROP TABLE plugin_notice;
DROP TABLE plugin_timer;
DROP TABLE plugin_asset;
DELETE FROM channel WHERE ty = 'plugin';
ALTER TABLE channel DROP COLUMN plugin_type;
-- A value cannot be taken out of an enum type; `plugin` stays in `channel_type`, unused.
