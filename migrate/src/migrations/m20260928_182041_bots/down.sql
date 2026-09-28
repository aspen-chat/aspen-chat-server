UPDATE deployment_role SET permissions = permissions & ~32;
UPDATE community_role SET permissions = permissions & ~2048;
ALTER TABLE community_role
    DROP CONSTRAINT community_role_one_per_bot,
    DROP COLUMN bot;
DROP TABLE bot_token;
ALTER TABLE "user"
    DROP CONSTRAINT user_bot_public_is_bot,
    DROP CONSTRAINT user_bot_owner_is_bot,
    DROP COLUMN bot_public,
    DROP COLUMN bot_owner,
    DROP COLUMN bot;
