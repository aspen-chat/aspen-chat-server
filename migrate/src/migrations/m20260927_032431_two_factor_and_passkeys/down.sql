DROP TABLE recovery_code;
DROP TABLE passkey;
DROP TABLE totp_secret;
ALTER TABLE refresh_token DROP COLUMN verified_at;
