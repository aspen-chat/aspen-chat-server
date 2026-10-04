-- Community permissions take bits 0 to 31 and channel permissions bits 32 to 62
-- (`app::permissions::Permissions`), leaving each group room to grow. This moves the channel
-- permissions from bits 16 to 30 up to bits 32 to 46 wherever permissions are stored: roles
-- hold both groups, overrides only channel permissions. The community permissions, in bits 0
-- to 15, stay where they are.
UPDATE community_role SET permissions =
    (permissions & 65535) | ((permissions & 2147418112) << 16);
UPDATE channel_override SET
    allow = (allow & 65535) | ((allow & 2147418112) << 16),
    deny = (deny & 65535) | ((deny & 2147418112) << 16);
UPDATE category_override SET
    allow = (allow & 65535) | ((allow & 2147418112) << 16),
    deny = (deny & 65535) | ((deny & 2147418112) << 16);
