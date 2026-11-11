-- The list of an author's held messages (`app::message::held::read_held`) pages by id, a
-- UUIDv7 made when each was held, so a page's cursor needs no row to still be there. Those
-- scheduled for later are listed apart, through `held_message_scheduled`.
CREATE INDEX held_message_list ON held_message (author, id) WHERE send_at IS NULL;
