DROP INDEX plugin_timer_by_scope;
ALTER TABLE plugin_timer
    DROP CONSTRAINT plugin_timer_scope_whole,
    DROP COLUMN scope,
    DROP COLUMN scope_kind;
