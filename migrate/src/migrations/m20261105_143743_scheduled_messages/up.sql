-- A message scheduled for later is a held message (`app::message::scheduled`): it waits for
-- `send_at` (`waiting = 'scheduled'`), when its `releaseHeldMessage` job comes due and posts it
-- as any held message is posted. `send_at` stays set while it then waits for its channel's mana
-- (`waiting = 'mana'`), which marks it as one the queue never drops for waiting too long. One
-- that could not be posted is kept for its author with why (`failure`), since `failed_at`,
-- until they schedule it again or delete it, or it has been kept as long as failed ones are.
-- `stuck_since` is when one first could not go forward for a reason that may pass: due, but
-- with no room in its channel's queue; or paid for by the queue, but not posted.
ALTER TABLE held_message
    DROP CONSTRAINT held_message_waiting_check,
    ADD CONSTRAINT held_message_waiting_check
        CHECK (waiting IN ('preview', 'mana', 'scheduled')),
    ADD COLUMN send_at TIMESTAMPTZ,
    ADD COLUMN failure TEXT,
    ADD COLUMN failed_at TIMESTAMPTZ,
    ADD COLUMN stuck_since TIMESTAMPTZ,
    ADD CONSTRAINT held_message_scheduled_send_at
        CHECK (waiting <> 'scheduled' OR send_at IS NOT NULL),
    ADD CONSTRAINT held_message_failure
        CHECK ((failure IS NULL) = (failed_at IS NULL)
               AND (failure IS NULL OR waiting = 'scheduled'));

-- An author's scheduled messages in the order they are due: their list, the count their limit
-- is checked against, and the ones due before another of theirs.
CREATE INDEX held_message_scheduled ON held_message (author, send_at, id)
    WHERE send_at IS NOT NULL;

-- The failed ones, oldest first, which the sweep removes once they have been kept long enough.
CREATE INDEX held_message_failed ON held_message (failed_at) WHERE failed_at IS NOT NULL;

-- A pool's queue in the order sent holds only what may be dropped for waiting too long, so the
-- drainer reads no scheduled message it must leave.
DROP INDEX held_message_mana_by_age;
CREATE INDEX held_message_mana_by_age ON held_message (channel, held_at, id)
    WHERE waiting = 'mana' AND send_at IS NULL;
