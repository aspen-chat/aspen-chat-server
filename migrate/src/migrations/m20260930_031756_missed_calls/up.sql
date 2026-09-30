-- Whether a call ever held two people at once; one that never did is recorded as missed.
ALTER TABLE voice_session ADD COLUMN had_company BOOLEAN NOT NULL DEFAULT false;

-- The system message recording a DM's call that no one else joined.
ALTER TYPE message_kind ADD VALUE 'missed_call';
