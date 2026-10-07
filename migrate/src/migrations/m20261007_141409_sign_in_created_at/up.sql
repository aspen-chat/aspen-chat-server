-- When each sign-in began, which an event stream compares with when an end of sign-ins or a ban
-- was published, so one retained from before the sign-in began does not end it
-- (`app::event_feed`). Sign-ins made before this column existed read as the earliest time, so
-- every such event still covers them.
ALTER TABLE refresh_token ADD COLUMN created_at TIMESTAMPTZ NOT NULL DEFAULT '1970-01-01T00:00:00Z';
ALTER TABLE refresh_token ALTER COLUMN created_at SET DEFAULT now();
