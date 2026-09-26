ALTER TABLE "user"
    DROP CONSTRAINT user_status_emoji_needs_text,
    DROP COLUMN status_emoji,
    DROP COLUMN status_text,
    DROP COLUMN bio,
    DROP COLUMN pronouns,
    DROP COLUMN display_name;
