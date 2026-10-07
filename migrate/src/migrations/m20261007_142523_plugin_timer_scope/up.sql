-- The scope a plugin's timer was set in (`set-timer-in`): a community, a channel, or a user,
-- named as `plugin_storage` names its scopes. A timer with a scope is deleted with what the plugin
-- keeps there when that scope goes; one without lives until it falls due or is cancelled.
ALTER TABLE plugin_timer
    ADD COLUMN scope_kind TEXT CHECK (scope_kind IN ('community', 'channel', 'user')),
    ADD COLUMN scope UUID,
    ADD CONSTRAINT plugin_timer_scope_whole CHECK ((scope_kind IS NULL) = (scope IS NULL));
CREATE INDEX plugin_timer_by_scope ON plugin_timer (scope_kind, scope) WHERE scope IS NOT NULL;
