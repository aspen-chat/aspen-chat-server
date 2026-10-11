DROP INDEX mention_everyone_recent;
DROP INDEX mention_role_recent;
DROP INDEX mention_user_recent;
DROP TRIGGER mention_inserted ON mention;
DROP FUNCTION mention_inserted();
ALTER TABLE mention DROP COLUMN home_channel;
