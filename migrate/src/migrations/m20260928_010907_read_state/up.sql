-- How far each user has read each channel. `message` is a position among the channel's message
-- ids rather than a reference: ids are UUIDv7 and ordered by time, so everything after it is
-- unread whether or not the message it names still exists.
CREATE TABLE read_state (
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    channel UUID NOT NULL REFERENCES channel (id) ON DELETE CASCADE,
    message UUID NOT NULL,
    PRIMARY KEY ("user", channel)
);

-- When a member joined, before which nothing in the community's channels is unread to them.
-- Members from before this column count from when it was added.
ALTER TABLE community_user ADD COLUMN joined_at TIMESTAMPTZ NOT NULL DEFAULT now();
