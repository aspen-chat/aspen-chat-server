UPDATE category_override SET allow = allow & ~536870912, deny = deny & ~536870912;
UPDATE channel_override SET allow = allow & ~536870912, deny = deny & ~536870912;
UPDATE community_role SET permissions = permissions & ~536870912;
