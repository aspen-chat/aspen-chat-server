-- Held messages by the channel they wait in: deleting a channel cascades to them, and a thread
-- made by a held first reply is removed with it only while nothing else waits in it.
CREATE INDEX held_message_by_channel ON held_message (channel);
