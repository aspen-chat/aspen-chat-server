-- This file should undo anything in `up.sql`




ALTER TABLE "message" DROP COLUMN "content";


ALTER TABLE "user" DROP COLUMN "password_hash";

