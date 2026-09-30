DELETE FROM "user" WHERE system;
DROP INDEX user_name_key;
CREATE UNIQUE INDEX user_name_key ON "user" (name) WHERE home_domain IS NULL;
DROP INDEX user_system;
ALTER TABLE "user" DROP CONSTRAINT user_system_own, DROP COLUMN system;
ALTER TABLE community DROP COLUMN everyone_limited_at;
