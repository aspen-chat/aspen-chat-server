-- The channel permissions return to bits 16 to 30, dropping any community permission above
-- bit 15, which that layout has no room for.
UPDATE category_override SET
    allow = (allow & 65535) | ((allow >> 16) & 2147418112),
    deny = (deny & 65535) | ((deny >> 16) & 2147418112);
UPDATE channel_override SET
    allow = (allow & 65535) | ((allow >> 16) & 2147418112),
    deny = (deny & 65535) | ((deny >> 16) & 2147418112);
UPDATE community_role SET permissions =
    (permissions & 65535) | ((permissions >> 16) & 2147418112);
