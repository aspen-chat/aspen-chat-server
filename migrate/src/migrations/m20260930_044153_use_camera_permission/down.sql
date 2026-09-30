UPDATE community_role SET permissions = permissions & ~1073741824;
UPDATE channel_override SET allow = allow & ~1073741824, deny = deny & ~1073741824;
UPDATE category_override SET allow = allow & ~1073741824, deny = deny & ~1073741824;
