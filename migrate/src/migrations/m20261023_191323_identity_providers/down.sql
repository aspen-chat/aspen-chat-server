UPDATE deployment_role SET permissions = permissions & ~(1::BIGINT << 16);
DELETE FROM job WHERE kind = 'forgetIdentityProvider';
ALTER TABLE refresh_token DROP COLUMN identity_provider_session, DROP COLUMN identity_provider;
DROP TABLE user_identity;
DROP TABLE identity_provider;
