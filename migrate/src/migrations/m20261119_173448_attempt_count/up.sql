-- Counts of what one account may do only so often (`app::attempt_count`): wrong passwords and
-- codes in a quarter hour and in a day, and reset codes mailed in an hour. Each window starts with
-- its first count and ends at `until`, on the database's clock; a count past its `until` starts
-- again, and `sweepExpired` deletes it through `attempt_count_until`. They are here and not in
-- Valkey because losing one would hand a guesser a fresh allowance.
CREATE TABLE attempt_count (
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    count INTEGER NOT NULL,
    until TIMESTAMPTZ NOT NULL,
    PRIMARY KEY ("user", kind)
);

CREATE INDEX attempt_count_until ON attempt_count (until);
