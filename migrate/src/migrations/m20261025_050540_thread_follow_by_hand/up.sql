-- Whether the user followed the thread by hand (`app::thread_follow::follow`) rather than by
-- taking part in it: a follow made by taking part never lets one made by hand go, so automatic
-- follows past the cap are let go oldest first through `thread_follow_automatic_by_age`. Follows
-- made before this column count as automatic, since which they were is not known.
ALTER TABLE thread_follow ADD COLUMN by_hand boolean NOT NULL DEFAULT false;
CREATE INDEX thread_follow_automatic_by_age ON thread_follow ("user", followed_at)
    WHERE NOT by_hand;
