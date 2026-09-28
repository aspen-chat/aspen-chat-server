UPDATE category_override SET allow = allow & ~(67108864 | 134217728 | 268435456),
    deny = deny & ~(67108864 | 134217728 | 268435456);
UPDATE channel_override SET allow = allow & ~(67108864 | 134217728 | 268435456),
    deny = deny & ~(67108864 | 134217728 | 268435456);
UPDATE community_role SET permissions = permissions & ~(67108864 | 134217728 | 268435456);
DROP TABLE mention;
ALTER TABLE message DROP COLUMN mentions;
