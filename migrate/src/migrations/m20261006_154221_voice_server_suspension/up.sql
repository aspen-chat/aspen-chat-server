-- Until when a voice server is left out of join offers because people kept failing to reach
-- it (`app::voice::report_failure`). Unlike `enabled`, which only an operator changes, it lapses
-- on its own, and an operator enabling the server clears it.
ALTER TABLE voice_server ADD COLUMN suspended_until TIMESTAMPTZ;
