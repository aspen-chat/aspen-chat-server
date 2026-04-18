-- Supports bidirectional cursor pagination over (channel, id) used by
-- the Before/After/Around variants of ChannelMessagesRead so individual
-- channels with very large message histories stay O(log n) to page.
CREATE INDEX message_channel_id_idx ON message (channel, id);
