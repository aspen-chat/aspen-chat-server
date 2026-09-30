-- A call in a DM or group DM rings everyone in it who is not in the call when it starts, until
-- they join or decline it or `until` passes; it ends with its session, or its user.
CREATE TABLE voice_ring (
    session UUID NOT NULL REFERENCES voice_session (id) ON DELETE CASCADE,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    -- Who started the call.
    caller UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    rung_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    until TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (session, "user")
);
CREATE INDEX voice_ring_by_user ON voice_ring ("user");

-- Who started a call: the first to be in it, whom the message recording it names.
ALTER TABLE voice_session
    ADD COLUMN started_by UUID REFERENCES "user" (id) ON DELETE SET NULL;

-- A system message recording that a DM's call ended, and how long it lasted.
ALTER TYPE message_kind ADD VALUE 'call';
ALTER TABLE message ADD COLUMN call_seconds INTEGER;
