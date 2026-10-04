ALTER TABLE "user" DROP COLUMN name_hue;
ALTER TABLE deployment_role DROP COLUMN hue;
ALTER TABLE community_role
    DROP CONSTRAINT community_role_everyone_plain,
    DROP COLUMN hoist,
    DROP COLUMN hue;
