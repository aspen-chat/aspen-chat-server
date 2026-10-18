-- A notification level for a category (`app::notification_setting`), between a channel's own and
-- its community's: a channel without a setting of its own takes its category's, then its
-- community's. Each row is about exactly one of a community, a category, or a channel.
ALTER TABLE notification_setting
    ADD COLUMN category UUID REFERENCES category (id) ON DELETE CASCADE;
ALTER TABLE notification_setting DROP CONSTRAINT notification_setting_check;
ALTER TABLE notification_setting
    ADD CONSTRAINT notification_setting_one_target CHECK (num_nonnulls(community, category, channel) = 1);
CREATE UNIQUE INDEX notification_setting_category ON notification_setting ("user", category)
    WHERE category IS NOT NULL;
-- Who wants every message of a category's channels, for waking them; and the category's rows,
-- which deleting it removes.
CREATE INDEX notification_setting_all_category ON notification_setting (category)
    WHERE level = 'all' AND category IS NOT NULL;
CREATE INDEX notification_setting_by_category ON notification_setting (category)
    WHERE category IS NOT NULL;

-- Categories a user has muted, for themself alone (`app::category_mute`): every channel in the
-- category is muted for them, those moved in later too, unless the channel's own row says
-- otherwise. As with `channel_mute`, a mute with no `until` lasts until it is lifted, and one
-- whose `until` has passed is over whether or not its row has been removed yet; a row is
-- replaced by the user's next mute of the category and removed with the category or the user.
CREATE TABLE category_mute (
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    category UUID NOT NULL REFERENCES category (id) ON DELETE CASCADE,
    until TIMESTAMPTZ,
    PRIMARY KEY ("user", category)
);
CREATE INDEX category_mute_category ON category_mute (category);

-- A channel's own row outranks its category's mute: `muted` true mutes it, false keeps it
-- unmuted while its category is muted, which lasts until the user changes it.
ALTER TABLE channel_mute ADD COLUMN muted BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE channel_mute
    ADD CONSTRAINT channel_mute_kept_unmuted_lasts CHECK (muted OR until IS NULL);
