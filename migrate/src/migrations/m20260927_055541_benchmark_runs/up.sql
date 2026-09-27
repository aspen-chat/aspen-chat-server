-- A benchmark run's seeded population, so `bench purge` can remove it with everything that came
-- to depend on it. `plan` is the seed plan as given.
CREATE TABLE benchmark_run (
    run TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    plan JSONB NOT NULL
);

CREATE TABLE benchmark_user (
    run TEXT NOT NULL REFERENCES benchmark_run (run),
    "user" UUID NOT NULL REFERENCES "user" (id),
    PRIMARY KEY (run, "user")
);

CREATE TABLE benchmark_community (
    run TEXT NOT NULL REFERENCES benchmark_run (run),
    community UUID NOT NULL REFERENCES community (id),
    PRIMARY KEY (run, community)
);
