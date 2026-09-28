-- Channels a user has muted, for themself alone. A mute with no `until` lasts until the user
-- lifts it; one whose `until` has passed is over, whether or not its row has been removed yet.
CREATE TABLE channel_mute (
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    channel UUID NOT NULL REFERENCES channel (id) ON DELETE CASCADE,
    until TIMESTAMPTZ,
    PRIMARY KEY ("user", channel)
);
