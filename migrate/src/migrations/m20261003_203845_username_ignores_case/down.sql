DROP INDEX user_name_key;
CREATE UNIQUE INDEX user_name_key ON "user" (name) WHERE home_domain IS NULL AND NOT system;
