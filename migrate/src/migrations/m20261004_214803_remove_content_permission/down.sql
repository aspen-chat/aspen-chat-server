UPDATE deployment_role SET permissions = permissions | (1::BIGINT << 5)
    WHERE permissions & (1::BIGINT << 11) <> 0;

UPDATE deployment_role SET permissions = permissions & ~(1::BIGINT << 13);
