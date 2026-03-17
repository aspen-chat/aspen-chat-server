-- This file should undo anything in `up.sql`
ALTER TABLE message DROP COLUMN timestamp;
ALTER TABLE message ADD COLUMN time TIMESTAMP NOT NULL;
ALTER TABLE attachment DROP COLUMN timestamp;
ALTER TABLE icon DROP COLUMN timestamp;
ALTER TABLE react DROP COLUMN timestamp;
ALTER TABLE "user" DROP COLUMN created_at, DROP COLUMN last_seen_at;