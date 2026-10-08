-- Each person's reactions, oldest first, carrying what was used: what their most used emoji
-- are counted from (`app::react::read_frequent`) without reading the table, and what finds
-- their reactions for the foreign key to `"user"`.
CREATE INDEX react_by_author ON react (author, "timestamp") INCLUDE (emoji, custom_emoji);
DROP INDEX react_author;
