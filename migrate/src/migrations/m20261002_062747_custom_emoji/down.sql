UPDATE community_role SET permissions = permissions & ~4096;
DROP INDEX react_by_custom_emoji;
ALTER TABLE react DROP COLUMN custom_emoji;
DROP TABLE custom_emoji;
