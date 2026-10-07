DROP INDEX plugin_timer_by_owner;
ALTER TABLE plugin_timer DROP COLUMN owner, DROP COLUMN owner_kind;
DROP TABLE plugin_storage_usage;
