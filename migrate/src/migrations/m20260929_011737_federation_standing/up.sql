-- A foreign user's standing: when their home last said they may be here, and whether this
-- deployment's moderators banned them from it.
ALTER TABLE "user"
    ADD COLUMN home_confirmed_at TIMESTAMPTZ,
    ADD COLUMN banned_at TIMESTAMPTZ,
    ADD COLUMN banned_by UUID REFERENCES "user" (id) ON DELETE SET NULL,
    ADD CONSTRAINT user_ban_is_foreign CHECK (banned_at IS NULL OR home_domain IS NOT NULL),
    ADD CONSTRAINT user_ban_by_needs_ban CHECK (banned_at IS NOT NULL OR banned_by IS NULL);

-- Foreign users arrived before now have been confirmed by arriving.
UPDATE "user" SET home_confirmed_at = created_at WHERE home_domain IS NOT NULL;

CREATE INDEX user_home_confirmed ON "user" (home_domain, home_confirmed_at)
    WHERE home_domain IS NOT NULL AND deleted_at IS NULL;
