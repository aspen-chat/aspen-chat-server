DROP INDEX user_home_confirmed;
ALTER TABLE "user"
    DROP CONSTRAINT user_ban_by_needs_ban,
    DROP CONSTRAINT user_ban_is_foreign,
    DROP COLUMN banned_by,
    DROP COLUMN banned_at,
    DROP COLUMN home_confirmed_at;
