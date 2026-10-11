-- The channel a tag is read in, as `message.home_channel` is for its message: its own, or,
-- for a thread reply, the thread's parent, so a place's tags of everyone are found with its
-- threads' through one index.
ALTER TABLE mention ADD COLUMN home_channel UUID;
UPDATE mention SET home_channel = m.home_channel FROM message m WHERE m.id = mention.message;
ALTER TABLE mention ALTER COLUMN home_channel SET NOT NULL;
CREATE FUNCTION mention_inserted() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    SELECT home_channel INTO NEW.home_channel FROM message WHERE id = NEW.message;
    RETURN NEW;
END
$$;
CREATE TRIGGER mention_inserted BEFORE INSERT ON mention
    FOR EACH ROW EXECUTE FUNCTION mention_inserted();

-- The newest messages that tag someone, a role, or everyone in a place, read backwards from
-- the newest: what the activity feed's page reads (`app::activity`), as far as the page and no
-- further. `mention_by_user` and `mention_by_role` lead with the channel instead, for counting
-- one channel's unread tags.
CREATE INDEX mention_user_recent ON mention (target_user, message) WHERE target_user IS NOT NULL;
CREATE INDEX mention_role_recent ON mention (target_role, message) WHERE target_role IS NOT NULL;
CREATE INDEX mention_everyone_recent ON mention (home_channel, message) WHERE everyone;
