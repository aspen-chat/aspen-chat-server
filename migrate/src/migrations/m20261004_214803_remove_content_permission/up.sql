-- Remove content (bit 13), deleting what a report case or a ban names, goes to every role that
-- holds Moderate any community (bit 4), which includes it, so no moderator loses an action.
UPDATE deployment_role SET permissions = permissions | (1::BIGINT << 13)
    WHERE permissions & (1::BIGINT << 4) <> 0;

-- Bit 5 names no permission: deleting ownerless bots belongs to Manage deployment settings
-- (bit 11). A role holding bit 5 without it stops deleting them rather than gaining every
-- setting.
UPDATE deployment_role SET permissions = permissions & ~(1::BIGINT << 5);
