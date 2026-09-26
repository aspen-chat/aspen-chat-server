ALTER TABLE "user"
    ADD COLUMN display_name TEXT,
    ADD COLUMN pronouns TEXT,
    ADD COLUMN bio TEXT,
    ADD COLUMN status_text TEXT,
    ADD COLUMN status_emoji TEXT,
    -- A status is its text; an emoji only decorates one.
    ADD CONSTRAINT user_status_emoji_needs_text CHECK (status_emoji IS NULL OR status_text IS NOT NULL);
