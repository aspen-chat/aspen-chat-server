ALTER TABLE "community" DROP COLUMN "icon";
ALTER TABLE "community" ADD COLUMN "icon_mime_type" TEXT;
ALTER TABLE "community" ADD COLUMN "icon" BYTEA;




ALTER TABLE "user" DROP COLUMN "icon";

-- `session.refresh_token` has a foreign key to `refresh_token(token)`,
-- so `session` must be dropped before `refresh_token`.
DROP TABLE IF EXISTS "other_server_auth_token";
DROP TABLE IF EXISTS "session";
DROP TABLE IF EXISTS "refresh_token";
DROP TABLE IF EXISTS "icon";
