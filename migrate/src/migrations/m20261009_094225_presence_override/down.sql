ALTER TABLE "user"
    DROP CONSTRAINT user_presence_override_until_needs_override,
    DROP COLUMN presence_override_until,
    DROP COLUMN presence_override;
