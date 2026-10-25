-- Each link keeps the issuer it was made at, and is matched by provider, issuer, and `sub`
-- together (`app::identity_provider::identity`): a `sub` is unique only within one issuer, so a
-- provider row that came to name another issuer must find none of the accounts linked before.
ALTER TABLE user_identity ADD COLUMN issuer TEXT;

UPDATE user_identity SET issuer = identity_provider.issuer
FROM identity_provider
WHERE identity_provider.id = user_identity.provider;

ALTER TABLE user_identity
    ALTER COLUMN issuer SET NOT NULL,
    ADD CONSTRAINT user_identity_issuer_length CHECK (length(issuer) BETWEEN 1 AND 2048);
