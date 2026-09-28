-- Federation: this deployment's signing keys, the other deployments it knows, and the lists
-- that decide which of them its users and bots may go to and come from.
DROP TABLE other_server_auth_token;

-- The keys this deployment signs its assertions with, Ed25519. The current one is the one not
-- retired; the server makes it on first start.
CREATE TABLE federation_key (
    id UUID PRIMARY KEY,
    -- PKCS #8.
    private_key BYTEA NOT NULL,
    -- The 32 bytes of the Ed25519 public key.
    public_key BYTEA NOT NULL CHECK (octet_length(public_key) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    retired_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX federation_key_current ON federation_key ((true)) WHERE retired_at IS NULL;

-- Every other deployment this one knows: added by an administrator, or recorded when it was
-- first contacted. `public_key` is the key it presented first, pinned; a different key it
-- presents later waits in `offered_key`, refused, until an administrator accepts it.
CREATE TABLE federated_deployment (
    -- Its domain, with a port when it is not served on 443, in lowercase.
    domain TEXT PRIMARY KEY,
    origin TEXT NOT NULL CHECK (origin IN ('administrator', 'terminal', 'firstContact')),
    added_by UUID REFERENCES "user" (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    note TEXT,
    public_key BYTEA CHECK (octet_length(public_key) = 32),
    first_contact_at TIMESTAMPTZ,
    last_contact_at TIMESTAMPTZ,
    offered_key BYTEA CHECK (octet_length(offered_key) = 32),
    offered_key_at TIMESTAMPTZ,
    CONSTRAINT federated_deployment_offer_needs_pin CHECK (offered_key IS NULL OR public_key IS NOT NULL),
    CONSTRAINT federated_deployment_offer_time CHECK ((offered_key IS NULL) = (offered_key_at IS NULL)),
    CONSTRAINT federated_deployment_contact_time CHECK ((public_key IS NULL) = (first_contact_at IS NULL))
);

CREATE INDEX federated_deployment_domain_trgm ON federated_deployment USING gin (domain gin_trgm_ops);

-- Which lists each deployment is on. A list is named by who it governs (users or bots), which
-- way (emigration, immigration, or the one list both directions share), and whether it allows
-- or blocks; `[federation]` in aspen.toml says which lists are in force, and the rest keep
-- their entries unread.
CREATE TABLE federation_list_entry (
    domain TEXT NOT NULL REFERENCES federated_deployment (domain) ON DELETE CASCADE,
    list TEXT NOT NULL CHECK (list IN (
        'usersEmigrationAllow', 'usersEmigrationBlock',
        'usersImmigrationAllow', 'usersImmigrationBlock',
        'usersSharedAllow', 'usersSharedBlock',
        'botsEmigrationAllow', 'botsEmigrationBlock',
        'botsImmigrationAllow', 'botsImmigrationBlock',
        'botsSharedAllow', 'botsSharedBlock'
    )),
    added_by UUID REFERENCES "user" (id) ON DELETE SET NULL,
    added_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (domain, list)
);

-- Manage federation (64) is part of administration: every role that holds all of it so far
-- (view the dashboard, registration invites, voice servers, deployment roles, and bots) gains
-- it.
UPDATE deployment_role SET permissions = permissions | 64 WHERE permissions & 47 = 47;
