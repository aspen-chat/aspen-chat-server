-- A poll's creator may let its voters add answers of their own.
ALTER TABLE poll ADD COLUMN allow_write_ins BOOLEAN NOT NULL DEFAULT false;

-- A written-in answer is an option like the creator's, after them in index order, recording
-- who wrote it (null once their account is gone). Removing one keeps its row, and so its
-- index, with `removed_at` set: votes are cast by index, and renumbering the answers after it
-- would move a vote already on its way onto a different answer.
ALTER TABLE poll_option
    ADD COLUMN write_in BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN written_by UUID REFERENCES "user" (id) ON DELETE SET NULL,
    ADD COLUMN removed_at TIMESTAMPTZ;

-- One standing write-in per person per poll.
CREATE UNIQUE INDEX poll_option_one_write_in
    ON poll_option (poll, written_by)
    WHERE write_in AND removed_at IS NULL;
