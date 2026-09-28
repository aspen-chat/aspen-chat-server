-- A message's reactions, each emoji's in the order they were made: what message reads
-- summarise and the reaction list pages through, earliest first.
CREATE INDEX react_by_message ON react (message, emoji, "timestamp", author);
