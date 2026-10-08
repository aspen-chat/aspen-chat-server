-- Secrets the deployment's servers share and nothing outside them knows, each made by the first
-- server to start that needs it (`app::server_secret`): `codes` keys the digests that mailed
-- codes are kept as in Valkey, so a read of Valkey cannot reverse them by trying every code.
CREATE TABLE server_secret (
    name TEXT PRIMARY KEY,
    secret BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
