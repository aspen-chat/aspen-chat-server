DROP TABLE user_foreign_deployment;
ALTER TABLE refresh_token DROP COLUMN method;
DELETE FROM "user" WHERE home_domain IS NOT NULL;
DROP INDEX user_home;
DROP INDEX user_name_key;
ALTER TABLE "user" ADD CONSTRAINT user_name_key UNIQUE (name);
ALTER TABLE "user"
    DROP CONSTRAINT user_home_whole,
    DROP COLUMN home_icon,
    DROP COLUMN home_id,
    DROP COLUMN home_domain;
ALTER TABLE federation_key DROP COLUMN handover;
