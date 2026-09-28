ALTER TABLE "user" ADD COLUMN admin BOOLEAN NOT NULL DEFAULT false;
UPDATE "user" SET admin = true
WHERE id IN (SELECT "user" FROM user_deployment_role);
DROP TABLE moderation_log;
DROP TABLE user_deployment_role;
DROP TABLE deployment_role;
