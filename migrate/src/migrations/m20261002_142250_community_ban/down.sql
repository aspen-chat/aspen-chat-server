UPDATE community_role SET permissions = permissions & ~8192;
DROP TABLE community_ban;
