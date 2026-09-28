-- When an invite's last use was taken, for how long ago it stopped working. Invites already
-- used up take the time their last account was made, or now when none can be found.
ALTER TABLE registration_invite ADD COLUMN used_up_at TIMESTAMPTZ;
UPDATE registration_invite ri
SET used_up_at = COALESCE(
    (SELECT max(u.created_at) FROM "user" u WHERE u.registered_with = ri.code),
    now()
)
WHERE ri.uses >= ri.max_uses;
