-- Transfer files (1 << 29, 536870912) goes wherever Share screen (1 << 25, 33554432) does, which
-- every template gives: roles that hold it, and overrides that allow or deny it.
UPDATE community_role SET permissions = permissions | 536870912
    WHERE permissions & 33554432 = 33554432;
UPDATE channel_override SET allow = allow | 536870912 WHERE allow & 33554432 = 33554432;
UPDATE channel_override SET deny = deny | 536870912 WHERE deny & 33554432 = 33554432;
UPDATE category_override SET allow = allow | 536870912 WHERE allow & 33554432 = 33554432;
UPDATE category_override SET deny = deny | 536870912 WHERE deny & 33554432 = 33554432;
