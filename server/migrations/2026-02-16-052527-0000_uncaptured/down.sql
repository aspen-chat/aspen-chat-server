-- This file should undo anything in `up.sql`


ALTER TABLE "community" DROP COLUMN "icon";
ALTER TABLE "community" ADD COLUMN "icon_mime_type" TEXT;
ALTER TABLE "community" ADD COLUMN "icon" BYTEA;




ALTER TABLE "user" DROP COLUMN "icon";

DROP TABLE IF EXISTS "other_server_auth_token";
DROP TABLE IF EXISTS "refresh_token";
DROP TABLE IF EXISTS "icon";
DROP TABLE IF EXISTS "session";
