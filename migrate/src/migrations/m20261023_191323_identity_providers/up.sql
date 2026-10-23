-- OpenID Connect providers people sign in through (`app::identity_provider`), the identities
-- they link to accounts, and which provider made each sign-in, for back-channel logout.

-- A removed provider keeps its row (`removed_at`), so the sign-ins it made keep naming it; its
-- links are deleted by a job (`forgetIdentityProvider`) a batch at a time.
CREATE TABLE identity_provider (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    -- As the provider's discovery document names it.
    issuer TEXT NOT NULL CHECK (length(issuer) BETWEEN 1 AND 2048),
    client_id TEXT NOT NULL CHECK (length(client_id) BETWEEN 1 AND 1024),
    -- Shown to no one once saved. Like the federation key, it makes the database's backups as
    -- sensitive as itself.
    client_secret TEXT NOT NULL CHECK (length(client_secret) BETWEEN 1 AND 1024),
    enabled BOOLEAN NOT NULL DEFAULT true,
    -- A first sign-in makes an account without a registration invite where one is required.
    skip_invite BOOLEAN NOT NULL DEFAULT false,
    -- Every sign-in there counts as two factors, whatever its `amr` claim says.
    enforces_two_factor BOOLEAN NOT NULL DEFAULT false,
    -- An address it reports as verified becomes a new account's verified address.
    trust_email BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    removed_at TIMESTAMPTZ
);

-- One provider of an issuer at a time; a removed one's issuer may be added again.
CREATE UNIQUE INDEX identity_provider_issuer ON identity_provider (issuer) WHERE removed_at IS NULL;

-- An identity at a provider, linked to the one account it signs in. Matched by the provider and
-- its `sub` alone, never by email address.
CREATE TABLE user_identity (
    id UUID PRIMARY KEY,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    provider UUID NOT NULL REFERENCES identity_provider (id),
    subject TEXT NOT NULL CHECK (length(subject) BETWEEN 1 AND 255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    UNIQUE (provider, subject),
    -- An account holds one identity of each provider.
    UNIQUE ("user", provider)
);

-- Which provider made a sign-in, and its session id there (`sid`) when it sent one, so the
-- provider's back-channel logout ends exactly those sign-ins. Providers are never deleted, so
-- the reference needs no action on delete.
ALTER TABLE refresh_token
    ADD COLUMN identity_provider UUID REFERENCES identity_provider (id),
    ADD COLUMN identity_provider_session TEXT;

CREATE INDEX refresh_token_identity_provider ON refresh_token (identity_provider, identity_provider_session)
    WHERE identity_provider IS NOT NULL;

-- Managing identity providers is a deployment permission of its own (bit 16); roles that manage
-- the deployment's settings are given it.
UPDATE deployment_role SET permissions = permissions | (1::BIGINT << 16)
WHERE permissions & (1::BIGINT << 11) <> 0;
