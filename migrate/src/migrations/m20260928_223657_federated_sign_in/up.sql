-- Signing in abroad: users of other deployments as local rows, how each sign-in proved who
-- its user is, the deployments each user has used, and key handovers.

-- A statement by the key before this one, vouching for this one (`app::federation`), made when
-- a key is replaced as planned; `NULL` for the first key and for one made after a compromise.
ALTER TABLE federation_key ADD COLUMN handover TEXT;

-- A foreign user is a row naming their home deployment and their id there; the row's own id is
-- this deployment's. Usernames are unique among this deployment's own users only, since a
-- foreign user keeps the name their home gave them. `home_icon` is the home's id of the
-- avatar that `icon` is this deployment's copy of.
ALTER TABLE "user"
    ADD COLUMN home_domain TEXT,
    ADD COLUMN home_id UUID,
    ADD COLUMN home_icon UUID,
    ADD CONSTRAINT user_home_whole CHECK ((home_domain IS NULL) = (home_id IS NULL));
ALTER TABLE "user" DROP CONSTRAINT user_name_key;
CREATE UNIQUE INDEX user_name_key ON "user" (name) WHERE home_domain IS NULL;
CREATE UNIQUE INDEX user_home ON "user" (home_domain, home_id) WHERE home_domain IS NOT NULL;

-- How a sign-in proved who its user is: a password alone, a password and a second factor, a
-- passkey, or a bot's token. A foreign user's sign-in records what their home said.
ALTER TABLE refresh_token
    ADD COLUMN method TEXT NOT NULL DEFAULT 'password'
        CHECK (method IN ('password', 'secondFactor', 'passkey', 'token'));
-- Existing sign-ins of accounts holding a second factor could only have been made with one.
UPDATE refresh_token SET method = 'secondFactor'
WHERE EXISTS (SELECT 1 FROM totp_secret t WHERE t."user" = refresh_token."user" AND t.confirmed_at IS NOT NULL)
   OR EXISTS (SELECT 1 FROM passkey p WHERE p."user" = refresh_token."user");
ALTER TABLE refresh_token ALTER COLUMN method DROP DEFAULT;

-- The other deployments each of this deployment's users has signed in to, so their other
-- devices can find them.
CREATE TABLE user_foreign_deployment (
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    domain TEXT NOT NULL,
    first_used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY ("user", domain)
);
