-- Message search (`app::search`). Words are matched with the `simple` configuration, which
-- lowercases but neither stems nor drops stop words, so it treats every language alike. The
-- index is on the expression the search writes out literally, so the planner uses it.
CREATE INDEX message_search ON message USING gin (to_tsvector('simple', content))
    WHERE deleted_at IS NULL;
-- Searching by who wrote a message, newest first.
CREATE INDEX message_by_author ON message (author, id) WHERE deleted_at IS NULL;
