-- When this sign-in last proved who its user is: at sign-in, and again whenever the user
-- re-verifies. Changes to security settings require it to be recent.
ALTER TABLE refresh_token ADD COLUMN verified_at TIMESTAMPTZ NOT NULL DEFAULT now();

-- An authenticator app's shared secret (RFC 6238). It counts as a second factor only once
-- `confirmed_at` is set by a correct code. `last_used_step` is the time step of the last code
-- accepted, so a code cannot be used twice.
CREATE TABLE totp_secret (
    "user" UUID PRIMARY KEY REFERENCES "user" (id),
    secret BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ,
    last_used_step BIGINT
);

-- A WebAuthn credential. `credential` is the verifier's stored form, including the public key
-- and the signature counter; `credential_id` is copied out of it so a sign-in can find it.
CREATE TABLE passkey (
    id UUID PRIMARY KEY,
    "user" UUID NOT NULL REFERENCES "user" (id),
    credential_id BYTEA NOT NULL UNIQUE,
    credential JSONB NOT NULL,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ
);
CREATE INDEX passkey_user ON passkey ("user");

-- Single-use codes that stand in for a lost second factor. Only their SHA-256 digests are kept;
-- the codes are random enough that a slow hash adds nothing.
CREATE TABLE recovery_code (
    "user" UUID NOT NULL REFERENCES "user" (id),
    code_hash BYTEA NOT NULL,
    used_at TIMESTAMPTZ,
    PRIMARY KEY ("user", code_hash)
);
