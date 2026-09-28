-- Tagging. A message's effective tags, the ones its author was allowed to make, are kept on
-- the message as its record carries them (`app::mention::Mentions`), and once each in
-- `mention`, which the unread counts read: a reader's tags in a channel are found through the
-- index on their own id, each role they hold, or @everyone, however much of the channel they
-- have not read.
ALTER TABLE message
    ADD COLUMN mentions JSONB NOT NULL DEFAULT '{"users": [], "roles": [], "everyone": false}';

CREATE TABLE mention (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    message UUID NOT NULL REFERENCES message (id) ON DELETE CASCADE,
    channel UUID NOT NULL REFERENCES channel (id) ON DELETE CASCADE,
    target_user UUID REFERENCES "user" (id) ON DELETE CASCADE,
    target_role UUID REFERENCES community_role (id) ON DELETE CASCADE,
    everyone BOOLEAN NOT NULL DEFAULT false,
    CHECK (num_nonnulls(target_user, target_role) + everyone::int = 1)
);

CREATE INDEX mention_by_user ON mention (target_user, channel, message)
    WHERE target_user IS NOT NULL;
CREATE INDEX mention_by_role ON mention (target_role, channel, message)
    WHERE target_role IS NOT NULL;
CREATE INDEX mention_everyone ON mention (channel, message) WHERE everyone;
CREATE INDEX mention_of_message ON mention (message);

-- Mention members (1 << 26) goes wherever Send messages (1 << 17) does; Mention roles
-- (1 << 27) and Mention everyone (1 << 28) wherever Manage messages (1 << 8) does, which the
-- Moderator and Admin templates give.
UPDATE community_role SET permissions = permissions | 67108864
    WHERE permissions & 131072 = 131072;
UPDATE community_role SET permissions = permissions | 134217728 | 268435456
    WHERE permissions & 256 = 256;
UPDATE channel_override SET allow = allow | 67108864 WHERE allow & 131072 = 131072;
UPDATE category_override SET allow = allow | 67108864 WHERE allow & 131072 = 131072;
