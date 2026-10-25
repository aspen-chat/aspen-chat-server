-- Whether a held message made the thread it waits in, as a first reply held for its previews
-- (`app::message::held::post`): only then does dropping or cancelling it take the empty thread
-- with it, so no one's held message removes a thread someone else made.
ALTER TABLE held_message ADD COLUMN made_thread boolean NOT NULL DEFAULT false;

-- A held row waiting first in a thread nothing has been posted in made it.
UPDATE held_message h SET made_thread = true
FROM channel c
WHERE c.id = h.channel AND c.ty = 'thread'
  AND NOT EXISTS (SELECT 1 FROM message m WHERE m.channel = c.id)
  AND NOT EXISTS (
      SELECT 1 FROM held_message earlier
      WHERE earlier.channel = h.channel AND (earlier.held_at, earlier.id) < (h.held_at, h.id));
