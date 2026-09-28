-- Who each user has blocked. A block is its blocker's alone: the blocked user is never told,
-- and nothing about it is published beyond the blocker's own devices. Each blocker's list is
-- read by the primary key; `user_block_blocked` serves the checks made from the other side
-- (a DM refused because the other person blocked the sender).
CREATE TABLE user_block (
    blocker UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    blocked UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (blocker, blocked),
    CHECK (blocker <> blocked)
);

CREATE INDEX user_block_blocked ON user_block (blocked, blocker);
