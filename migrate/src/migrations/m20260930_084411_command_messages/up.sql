-- A command someone invoked, shown in the channel as their message naming the bot it went to
-- (`app::bot_command`); its content is the command as it was sent.
ALTER TYPE message_kind ADD VALUE 'command';
ALTER TABLE message ADD COLUMN command_bot UUID REFERENCES "user" (id) ON DELETE SET NULL;
