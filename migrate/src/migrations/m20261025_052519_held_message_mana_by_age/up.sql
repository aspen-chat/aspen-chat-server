-- A pool's queue in the order sent, whoever sent it: what has waited longest, which the drainer
-- drops past its limit (`app::mana::queue::expire`), without sorting every waiting row.
CREATE INDEX held_message_mana_by_age ON held_message (channel, held_at, id)
    WHERE waiting = 'mana';
