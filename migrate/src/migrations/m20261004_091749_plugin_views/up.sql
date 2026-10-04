-- A channel of a kind a plugin adds (`app::plugin`): its `plugin_type` names the plugin and the
-- kind (`org.example.forums:board`), and its contents are the plugin's.
ALTER TYPE channel_type ADD VALUE IF NOT EXISTS 'plugin';
ALTER TABLE channel ADD COLUMN plugin_type TEXT;

-- The files a plugin's views are served from, installed with it.
CREATE TABLE plugin_asset (
    plugin TEXT NOT NULL REFERENCES plugin (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    content_type TEXT NOT NULL,
    bytes BYTEA NOT NULL,
    PRIMARY KEY (plugin, path)
);

-- A plugin's timers, each due once. A server claims a due one until `claimed_until`, and deletes
-- it once the plugin has handled it; one whose handling failed is tried again once its claim
-- runs out, `attempts` times at most.
CREATE TABLE plugin_timer (
    plugin TEXT NOT NULL REFERENCES plugin (id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    due TIMESTAMPTZ NOT NULL,
    payload TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    claimed_until TIMESTAMPTZ,
    PRIMARY KEY (plugin, key)
);
CREATE INDEX plugin_timer_by_due ON plugin_timer (due);

-- What plugins told people of, kept a week for their phones to read.
CREATE TABLE plugin_notice (
    id UUID PRIMARY KEY,
    plugin TEXT NOT NULL REFERENCES plugin (id) ON DELETE CASCADE,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    channel UUID NOT NULL REFERENCES channel (id) ON DELETE CASCADE,
    message UUID REFERENCES message (id) ON DELETE SET NULL,
    text JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX plugin_notice_by_created ON plugin_notice (created_at);

-- People's private URLs to plugins' routes, by their secret, which is kept so a plugin can show
-- a person the same URL again.
CREATE TABLE plugin_capability (
    secret TEXT PRIMARY KEY,
    plugin TEXT NOT NULL REFERENCES plugin (id) ON DELETE CASCADE,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (plugin, "user", name)
);

-- A card a plugin's account put on its message, naming the plugin.
ALTER TABLE message ADD COLUMN card JSONB;
