-- When the call last had one participant or fewer; NULL while two or more are in it. A
-- call alone for a day is ended to free the voice server.
ALTER TABLE voice_session ADD COLUMN alone_since TIMESTAMPTZ;
UPDATE voice_session SET alone_since = created_at;
