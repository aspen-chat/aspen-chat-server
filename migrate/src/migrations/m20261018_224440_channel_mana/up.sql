-- Channel mana (`app::mana`): how fast a channel fills, spent by what is posted and earned back
-- over time. The deployment's rate is the most any channel recharges, in mana a second; its
-- capacity is any pool's that sets none of its own, and the most one may set; an attachment
-- costs what a glyph does this many times.
ALTER TABLE deployment_settings
    ADD COLUMN mana_max_recharge INTEGER NOT NULL DEFAULT 50 CHECK (mana_max_recharge > 0),
    ADD COLUMN mana_max INTEGER NOT NULL DEFAULT 20000 CHECK (mana_max > 0),
    ADD COLUMN mana_attachment_cost INTEGER NOT NULL DEFAULT 30 CHECK (mana_attachment_cost >= 0);

-- A channel's own, where it sets them; absent, the deployment's.
ALTER TABLE channel
    ADD COLUMN mana_recharge INTEGER CHECK (mana_recharge > 0),
    ADD COLUMN mana_max INTEGER CHECK (mana_max > 0);

-- A held message may wait for its channel's mana as well as for its previews, and what waits
-- for mana may be a slash command, an edit of a message, a poll, or the echo of a thread reply
-- to its parent. `channel` is the pool it pays from; `target` the message an edit changes or an
-- echo copies; `payload` a command's invocation, an edit's change, or a poll as it was asked
-- for (an edit's new text is in `content` too, for its author's apps to show); `cost` what it
-- spends, in mana.
ALTER TABLE held_message
    ADD COLUMN waiting TEXT NOT NULL DEFAULT 'preview' CHECK (waiting IN ('preview', 'mana')),
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'message'
        CHECK (kind IN ('message', 'command', 'edit', 'poll', 'echo')),
    ADD COLUMN target UUID REFERENCES message (id) ON DELETE CASCADE,
    ADD COLUMN payload JSONB,
    ADD COLUMN cost INTEGER NOT NULL DEFAULT 0 CHECK (cost >= 0),
    ADD CONSTRAINT held_message_kind_target CHECK ((kind IN ('edit', 'echo')) = (target IS NOT NULL)),
    ADD CONSTRAINT held_message_kind_payload
        CHECK ((kind IN ('command', 'edit', 'poll')) = (payload IS NOT NULL)),
    ADD CONSTRAINT held_message_waiting_kind CHECK (waiting = 'mana' OR kind = 'message');

-- A pool's queue, by author (each author's waiting messages are one item) and in the order sent.
CREATE INDEX held_message_mana_queue ON held_message (channel, author, held_at, id)
    WHERE waiting = 'mana';
CREATE INDEX held_message_target ON held_message (target) WHERE target IS NOT NULL;

-- Bypass channel mana (1 << 47) goes to every role that holds Manage messages (1 << 8), as the
-- Moderator and Admin templates give it.
UPDATE community_role SET permissions = permissions | 140737488355328
    WHERE permissions & 256 = 256;
