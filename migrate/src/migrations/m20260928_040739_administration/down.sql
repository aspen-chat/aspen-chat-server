DROP INDEX community_name_search;
DROP INDEX user_display_name_search;
DROP INDEX user_name_search;
DROP INDEX user_registered_with;
ALTER TABLE "user" DROP COLUMN registered_with;
DROP TABLE registration_invite;
ALTER TABLE "user" DROP COLUMN admin;
