-- Refresh and session tokens are kept only as the hex SHA-256 of the token
-- (`app::login::token_digest`), so a copy of the database signs no one in. Each stored token is
-- replaced by its digest, keeping every sign-in working, and each reference to a refresh token
-- follows it. A sign-in's id, the first half of its refresh token's digest, does not change.
ALTER TABLE session DROP CONSTRAINT session_refresh_token_fkey;
ALTER TABLE push_subscription DROP CONSTRAINT push_subscription_refresh_token_fkey;

UPDATE refresh_token SET token = encode(sha256(convert_to(token, 'UTF8')), 'hex');
UPDATE session SET
    token = encode(sha256(convert_to(token, 'UTF8')), 'hex'),
    refresh_token = encode(sha256(convert_to(refresh_token, 'UTF8')), 'hex');
UPDATE push_subscription
SET refresh_token = encode(sha256(convert_to(refresh_token, 'UTF8')), 'hex');

ALTER TABLE session ADD CONSTRAINT session_refresh_token_fkey
    FOREIGN KEY (refresh_token) REFERENCES refresh_token (token);
ALTER TABLE push_subscription ADD CONSTRAINT push_subscription_refresh_token_fkey
    FOREIGN KEY (refresh_token) REFERENCES refresh_token (token) ON DELETE CASCADE;
