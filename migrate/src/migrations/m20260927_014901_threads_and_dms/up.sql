-- New values are only added here; nothing below uses them, since a value added to an enum
-- cannot be used in the transaction that adds it.
ALTER TYPE channel_type ADD VALUE 'thread';
ALTER TYPE channel_type ADD VALUE 'dm';
ALTER TYPE channel_type ADD VALUE 'group_dm';
ALTER TYPE message_kind ADD VALUE 'thread_echo';

-- A thread is a channel under another channel, started from one of that channel's messages.
-- `reply_count` and `last_reply_at` summarise it for the message that started it.
-- `dm_key` names the two people of a one-to-one DM (their ids sorted and joined), so there is
-- one such DM per pair however many requests race to create it.
ALTER TABLE channel
    ADD COLUMN parent_channel UUID REFERENCES channel (id),
    ADD COLUMN starter_message UUID REFERENCES message (id),
    ADD COLUMN reply_count INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN last_reply_at TIMESTAMPTZ,
    ADD COLUMN dm_key TEXT;
CREATE UNIQUE INDEX channel_starter_message ON channel (starter_message);
CREATE UNIQUE INDEX channel_dm_key ON channel (dm_key);
CREATE INDEX channel_parent_channel ON channel (parent_channel);

-- `thread` is the thread a message started, kept on the message as well as the thread's
-- `starter_message` so a channel's history carries it without a join; both are written in the
-- transaction that creates the thread. `echo_of` is the thread reply a `thread_echo` message
-- shows in the parent channel; a reply is echoed at most once.
ALTER TABLE message
    ADD COLUMN thread UUID REFERENCES channel (id),
    ADD COLUMN echo_of UUID REFERENCES message (id);
CREATE UNIQUE INDEX message_echo_of ON message (echo_of);

-- The people in a DM or group DM.
CREATE TABLE dm_recipient (
    channel UUID NOT NULL REFERENCES channel (id) ON DELETE CASCADE,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    joined_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (channel, "user")
);
CREATE INDEX dm_recipient_user ON dm_recipient ("user");
