-- What may be used only once and must stay refused after any restart (`app::used_once`): a
-- redeemed device challenge, a logout token from an identity provider, a statement from another
-- deployment. A row is kept until what it names could no longer be accepted anyway, stamped on
-- the database's clock, and `sweepExpired` deletes it after that through `used_once_until`.
CREATE TABLE used_once (
    purpose TEXT NOT NULL,
    key BYTEA NOT NULL,
    until TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (purpose, key)
);

CREATE INDEX used_once_until ON used_once (until);
