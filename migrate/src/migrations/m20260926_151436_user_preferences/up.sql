-- Account-scoped user preferences: one JSON object per user, keys namespaced by the client
-- (`audio.input`, `look.theme`), synchronised to every device of the user through the
-- `userPreferencesChanged` event.
CREATE TABLE user_preferences (
    "user" UUID PRIMARY KEY REFERENCES "user" (id) ON DELETE CASCADE,
    "values" JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at TIMESTAMPTZ NOT NULL
);
