-- Manage plugins (bit 16), the community permission to turn plugins on and configure them
-- (`app::plugin`), goes to every role that holds Manage community (bit 0), as the Admin template
-- has it.
UPDATE community_role SET permissions = permissions | (1::BIGINT << 16)
    WHERE permissions & 1 <> 0;
