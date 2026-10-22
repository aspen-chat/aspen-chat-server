ALTER TABLE held_message DROP COLUMN tags_replied, DROP COLUMN reply_to;
DROP INDEX message_reply_to;
ALTER TABLE message DROP COLUMN tags_replied, DROP COLUMN reply_to;
