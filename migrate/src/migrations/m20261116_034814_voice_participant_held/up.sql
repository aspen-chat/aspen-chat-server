-- Whether a participant's call is on hold, as their voice server reports: a phone call has their
-- device, and nothing of theirs is sent or sent to them. Beside `muted` and `deafened`, which are
-- their own choices and stand through the hold.
ALTER TABLE voice_participant ADD COLUMN held BOOLEAN NOT NULL DEFAULT false;
