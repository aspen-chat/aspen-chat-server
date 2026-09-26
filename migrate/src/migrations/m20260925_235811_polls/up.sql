CREATE TYPE message_kind AS ENUM ('standard', 'poll', 'poll_closed');

CREATE TABLE poll (
    id UUID PRIMARY KEY,
    channel UUID NOT NULL REFERENCES channel (id) ON DELETE CASCADE,
    created_by UUID NOT NULL REFERENCES "user" (id),
    question TEXT NOT NULL,
    multiple_choice BOOLEAN NOT NULL,
    anonymous BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    closes_at TIMESTAMPTZ NOT NULL,
    closed_at TIMESTAMPTZ
);
-- The closer scans open polls by deadline; closed polls never match.
CREATE INDEX poll_open_closes_at ON poll (closes_at) WHERE closed_at IS NULL;

-- One row per option, in the order the creator listed them.
CREATE TABLE poll_option (
    poll UUID NOT NULL REFERENCES poll (id) ON DELETE CASCADE,
    index INTEGER NOT NULL,
    label TEXT NOT NULL,
    PRIMARY KEY (poll, index)
);

CREATE TABLE poll_vote (
    poll UUID NOT NULL,
    option_index INTEGER NOT NULL,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    timestamp TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (poll, option_index, "user"),
    FOREIGN KEY (poll, option_index) REFERENCES poll_option (poll, index) ON DELETE CASCADE
);

ALTER TABLE message
    ADD COLUMN kind message_kind NOT NULL DEFAULT 'standard',
    ADD COLUMN poll UUID REFERENCES poll (id) ON DELETE SET NULL;
-- Exactly one message of kind 'poll' shows each poll.
CREATE UNIQUE INDEX message_poll_shown_once ON message (poll) WHERE kind = 'poll';
