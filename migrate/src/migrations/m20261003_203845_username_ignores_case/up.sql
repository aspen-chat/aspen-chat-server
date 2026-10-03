-- Usernames are unique regardless of case, so a sign-in can find its account by the name
-- typed in any case (`app::login::try_login`). A deployment that already has two of its own
-- users whose names differ only by case cannot take the index until an administrator renames
-- one of them, so the migration names them and stops.
DO $$
DECLARE
    clashes TEXT;
BEGIN
    SELECT string_agg(names, '; ') INTO clashes FROM (
        SELECT string_agg(name, ', ' ORDER BY name) AS names
        FROM "user"
        WHERE home_domain IS NULL AND NOT system
        GROUP BY lower(name)
        HAVING count(*) > 1
    ) AS clash;
    IF clashes IS NOT NULL THEN
        RAISE EXCEPTION 'usernames that differ only by case: %. Rename all but one of each group, then run the migration again', clashes;
    END IF;
END
$$;

DROP INDEX user_name_key;
CREATE UNIQUE INDEX user_name_key ON "user" (lower(name)) WHERE home_domain IS NULL AND NOT system;
