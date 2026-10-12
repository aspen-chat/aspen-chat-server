-- Two arrays of counts added element by element, a missing element counting as none.
CREATE FUNCTION aspen_add_counts(bigint[], bigint[]) RETURNS bigint[]
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT ARRAY(
        SELECT COALESCE(a, 0) + COALESCE(b, 0)
        FROM unnest($1, $2) WITH ORDINALITY AS counts(a, b, i)
        ORDER BY i
    )
$$;

-- How long the database took, as the servers saw it (`app::database_latency`): for each step
-- of each range the dashboard charts, how many statements (`queries`) and waits for a pooled
-- connection (`pool_waits`) took how long, counted in the buckets of
-- `aspen_database::latency`. Every server adds what it counted each minute to the step that
-- minute falls in, of every step length (`step_seconds`), so a range is read as its rows and a
-- row's percentiles are the whole deployment's. `at` is when the step began. The recurring job
-- `sweepExpired` deletes the steps older than their range reaches, so the table stays a few
-- hundred rows.
CREATE TABLE database_latency (
    step_seconds INTEGER NOT NULL,
    at TIMESTAMPTZ NOT NULL,
    queries BIGINT[] NOT NULL,
    pool_waits BIGINT[] NOT NULL,
    PRIMARY KEY (step_seconds, at)
);
