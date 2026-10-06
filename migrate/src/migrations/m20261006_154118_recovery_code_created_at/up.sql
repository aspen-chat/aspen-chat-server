-- When each recovery code was issued, so a password reset by email can take away the codes
-- issued within the week it reaches back (`app::two_factor::remove_recent`). A set is issued
-- with an account's first second factor, so the codes held now are dated by the earliest
-- factor the account still holds: codes an intruder received with a factor they added are dated
-- by it. The codes of an account with no factor left (none should remain) are dated now.
ALTER TABLE recovery_code ADD COLUMN created_at TIMESTAMPTZ NOT NULL DEFAULT now();

UPDATE recovery_code r
SET created_at = least(
    (SELECT t.confirmed_at FROM totp_secret t WHERE t."user" = r."user"),
    (SELECT min(p.created_at) FROM passkey p WHERE p."user" = r."user")
)
WHERE EXISTS (
    SELECT 1 FROM totp_secret t WHERE t."user" = r."user" AND t.confirmed_at IS NOT NULL
) OR EXISTS (SELECT 1 FROM passkey p WHERE p."user" = r."user");
