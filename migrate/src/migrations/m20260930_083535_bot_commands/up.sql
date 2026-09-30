-- The commands a bot advertises, as it last published them (`app::bot_command`): one list
-- per bot, validated when published, read by clients to complete what a person types.
CREATE TABLE bot_command_list (
    bot UUID PRIMARY KEY REFERENCES "user" (id) ON DELETE CASCADE,
    commands JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
