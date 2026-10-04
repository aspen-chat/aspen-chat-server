-- Plugins the operator installed (`app::plugin`): the manifest as installed, the component, the
-- permissions the operator granted, where it runs (`everywhere`, or `optIn` for the communities
-- that turn it on), whether it is on, its place in the order intercepting plugins run in, and
-- the deployment's settings. A removed plugin keeps its row, and so its data, until the operator
-- purges it, which deletes the row and everything that refers to it.
CREATE TABLE plugin (
    id TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    manifest JSONB NOT NULL,
    component BYTEA,
    granted TEXT[] NOT NULL DEFAULT '{}',
    mode TEXT NOT NULL CHECK (mode IN ('everywhere', 'optIn')),
    enabled BOOLEAN NOT NULL DEFAULT false,
    position INTEGER NOT NULL,
    settings JSONB NOT NULL DEFAULT '{}',
    storage_bytes BIGINT NOT NULL DEFAULT 0,
    installed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    removed_at TIMESTAMPTZ,
    CONSTRAINT plugin_removed_has_no_component CHECK (removed_at IS NULL OR component IS NULL),
    CONSTRAINT plugin_removed_is_off CHECK (removed_at IS NULL OR NOT enabled)
);

-- A plugin's account, its principal: a bot no person owns, which acts only through the host.
ALTER TABLE "user" ADD COLUMN plugin TEXT REFERENCES plugin (id) ON DELETE SET NULL;
CREATE UNIQUE INDEX user_plugin_principal ON "user" (plugin) WHERE plugin IS NOT NULL;

-- A community's use of a plugin: whether it turned it on (an `everywhere` plugin runs whatever
-- this says) and its settings there.
CREATE TABLE community_plugin (
    community UUID NOT NULL REFERENCES community (id) ON DELETE CASCADE,
    plugin TEXT NOT NULL REFERENCES plugin (id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT false,
    settings JSONB NOT NULL DEFAULT '{}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by UUID REFERENCES "user" (id) ON DELETE SET NULL,
    PRIMARY KEY (community, plugin)
);
CREATE INDEX community_plugin_by_plugin ON community_plugin (plugin);

-- What plugins keep, by scope: the deployment (with the nil id), a community, a channel, or a
-- user. Deleting what a scope names deletes what plugins kept in it (`app::plugin::storage`).
CREATE TABLE plugin_storage (
    plugin TEXT NOT NULL REFERENCES plugin (id) ON DELETE CASCADE,
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('deployment', 'community', 'channel', 'user')),
    scope UUID NOT NULL,
    key TEXT NOT NULL,
    value BYTEA NOT NULL,
    PRIMARY KEY (plugin, scope_kind, scope, key)
);
CREATE INDEX plugin_storage_by_scope ON plugin_storage (scope_kind, scope);

-- What plugins say about messages and people, one of each kind per plugin and subject. `label`
-- and `detail` are keys of the plugin's messages with their arguments.
CREATE TABLE message_annotation (
    id UUID PRIMARY KEY,
    plugin TEXT NOT NULL REFERENCES plugin (id) ON DELETE CASCADE,
    message UUID NOT NULL REFERENCES message (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL CHECK (severity IN ('info', 'notice', 'warning')),
    label JSONB NOT NULL,
    detail JSONB,
    link TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (message, plugin, kind)
);
CREATE INDEX message_annotation_by_plugin ON message_annotation (plugin);

CREATE TABLE user_annotation (
    id UUID PRIMARY KEY,
    plugin TEXT NOT NULL REFERENCES plugin (id) ON DELETE CASCADE,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL CHECK (severity IN ('info', 'notice', 'warning')),
    label JSONB NOT NULL,
    detail JSONB,
    link TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE ("user", plugin, kind)
);
CREATE INDEX user_annotation_by_plugin ON user_annotation (plugin);

-- The plugins that rewrote a message's text, in the order they ran.
ALTER TABLE message ADD COLUMN altered_by TEXT[] NOT NULL DEFAULT '{}';

-- Manage plugins (1 << 14, 16384), the community permission to turn plugins on and configure
-- them, goes to every role that holds Manage community (1 << 0), as the Admin template does.
UPDATE community_role SET permissions = permissions | 16384 WHERE permissions & 1 = 1;

-- Manage plugins (1 << 11, 2048), the deployment permission over installed plugins' settings,
-- goes to every role holding the first four administrative permissions (15), as the terminal's
-- Administrator role does.
UPDATE deployment_role SET permissions = permissions | 2048 WHERE permissions & 15 = 15;
