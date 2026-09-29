-- What a user wants to be told of (`app::notification_setting`): for a whole community, or one
-- channel of it or DM, every message, only messages that tag them, or nothing. A channel's own
-- setting outranks its community's; without either, a DM tells of every message and a
-- community channel of tags.
CREATE TABLE notification_setting (
    id UUID PRIMARY KEY,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    community UUID REFERENCES community (id) ON DELETE CASCADE,
    channel UUID REFERENCES channel (id) ON DELETE CASCADE,
    level TEXT NOT NULL CHECK (level IN ('all', 'tags', 'nothing')),
    CHECK ((community IS NULL) <> (channel IS NULL))
);
CREATE UNIQUE INDEX notification_setting_community ON notification_setting ("user", community)
    WHERE community IS NOT NULL;
CREATE UNIQUE INDEX notification_setting_channel ON notification_setting ("user", channel)
    WHERE channel IS NOT NULL;
-- Who wants every message of a community or channel, for waking them.
CREATE INDEX notification_setting_all_community ON notification_setting (community)
    WHERE level = 'all' AND community IS NOT NULL;
CREATE INDEX notification_setting_all_channel ON notification_setting (channel)
    WHERE level = 'all' AND channel IS NOT NULL;
