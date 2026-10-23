UPDATE deployment_role SET permissions = permissions & ~(1::BIGINT << 16);
DELETE FROM job WHERE kind = 'forgetIdentityProvider';
DELETE FROM session WHERE refresh_token IN (SELECT token FROM refresh_token WHERE identity_provider IS NOT NULL);
DELETE FROM refresh_token WHERE identity_provider IS NOT NULL;
ALTER TABLE refresh_token DROP CONSTRAINT refresh_token_method_check,
    ADD CONSTRAINT refresh_token_method_check CHECK (method IN ('password', 'secondFactor', 'passkey', 'token'));
ALTER TABLE refresh_token DROP COLUMN identity_provider_session, DROP COLUMN identity_provider;
DROP TABLE user_identity;
DROP TABLE identity_provider;
