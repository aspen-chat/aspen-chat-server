-- Push (`app::push`, `spec/push.md`). The deployment's push key, which signs every push it
-- sends and which relays bind subscriptions to: a P-256 key pair, the private half as PKCS #8.
-- The current key is the one not retired.
CREATE TABLE push_key (
    id UUID PRIMARY KEY,
    private_key BYTEA NOT NULL,
    public_key BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    retired_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX push_key_current ON push_key ((true)) WHERE retired_at IS NULL;

-- A phone to wake for one sign-in: the endpoint it was given by its relay or UnifiedPush
-- distributor, and the keys it decrypts with (RFC 8291). It belongs to the sign-in, so it goes
-- when the sign-in does.
CREATE TABLE push_subscription (
    id UUID PRIMARY KEY,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    refresh_token TEXT NOT NULL UNIQUE REFERENCES refresh_token (token) ON DELETE CASCADE,
    endpoint TEXT NOT NULL,
    p256dh BYTEA NOT NULL,
    auth BYTEA NOT NULL,
    push_key UUID NOT NULL REFERENCES push_key (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX push_subscription_by_user ON push_subscription ("user");
