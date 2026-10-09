-- What a user has chosen to show of their presence in place of what their connections say
-- (`app::presence_override`), and until when; `NULL` until they change it.
ALTER TABLE "user"
    ADD COLUMN presence_override TEXT
        CHECK (presence_override IN ('invisible', 'away', 'doNotDisturb')),
    ADD COLUMN presence_override_until TIMESTAMPTZ,
    ADD CONSTRAINT user_presence_override_until_needs_override
        CHECK (presence_override IS NOT NULL OR presence_override_until IS NULL);
