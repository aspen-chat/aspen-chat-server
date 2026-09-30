-- When the community gained the member that turned Mention everyone off for its everyone
-- role (`app::everyone_limit`); a community is limited once, however its membership moves.
ALTER TABLE community ADD COLUMN everyone_limited_at TIMESTAMPTZ;

-- Communities that already have the default threshold's 200 members count as having passed
-- it, so the limit changes only communities that reach it after this migration.
UPDATE community SET everyone_limited_at = now()
WHERE id IN (
    SELECT community FROM community_user GROUP BY community HAVING count(*) >= 200
);

-- The deployment's own account (`app::system_account`), which sends people notices from the
-- deployment itself: at most one, never a bot or anyone else's user, and outside the
-- usernames people sign in and are found by, so its name takes none from them.
ALTER TABLE "user"
    ADD COLUMN system BOOLEAN NOT NULL DEFAULT false,
    ADD CONSTRAINT user_system_own CHECK (NOT system OR (NOT bot AND home_domain IS NULL));
CREATE UNIQUE INDEX user_system ON "user" (system) WHERE system;
DROP INDEX user_name_key;
CREATE UNIQUE INDEX user_name_key ON "user" (name) WHERE home_domain IS NULL AND NOT system;
