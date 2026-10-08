-- Messages a user saved for themself (`app::saved_message`). Each save has an id of its own,
-- a UUIDv7 of when it was made, which orders the list newest first and pages it. A message is
-- saved at most once per user; deleting the message deletes its saves.
CREATE TABLE saved_message (
    id UUID PRIMARY KEY,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    message UUID NOT NULL REFERENCES message (id) ON DELETE CASCADE,
    UNIQUE ("user", message)
);
CREATE INDEX saved_message_by_user ON saved_message ("user", id);
CREATE INDEX saved_message_by_message ON saved_message (message);

-- Threads a user follows (`app::thread_follow`), whose every reply tells them. `followed_at` is
-- when they last followed it, by hand or by taking part, which decides which follows go first
-- when they hold too many.
CREATE TABLE thread_follow (
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    thread UUID NOT NULL REFERENCES channel (id) ON DELETE CASCADE,
    followed_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY ("user", thread)
);
CREATE INDEX thread_follow_by_thread ON thread_follow (thread, "user");
CREATE INDEX thread_follow_by_age ON thread_follow ("user", followed_at);
