UPDATE community_role SET permissions = permissions & ~(1::BIGINT << 16);
