ALTER TABLE category
DROP COLUMN deleted_at;

ALTER TABLE channel
DROP COLUMN deleted_at;

ALTER TABLE message
DROP COLUMN deleted_at;

ALTER TABLE "user"
DROP COLUMN deleted_at;
