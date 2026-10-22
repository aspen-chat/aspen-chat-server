-- `reply_to` is the message a reply answers, in the same channel (`app::message::reply`), and
-- `tags_replied` whether the reply tags that message's author, which an edit that tags afresh
-- keeps. A reply outlives its original's deletion, which is soft; the reference is indexed for
-- the check its foreign key makes when a message is removed.
ALTER TABLE message
    ADD COLUMN reply_to UUID REFERENCES message (id),
    ADD COLUMN tags_replied BOOLEAN NOT NULL DEFAULT false;
CREATE INDEX message_reply_to ON message (reply_to) WHERE reply_to IS NOT NULL;

-- A message held for its previews is posted as it was sent, its reply included.
ALTER TABLE held_message
    ADD COLUMN reply_to UUID,
    ADD COLUMN tags_replied BOOLEAN NOT NULL DEFAULT false;
