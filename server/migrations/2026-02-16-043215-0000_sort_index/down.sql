-- This file should undo anything in `up.sql`

ALTER TABLE "category" DROP COLUMN sort_index;
ALTER TABLE "channel" DROP COLUMN sort_index;