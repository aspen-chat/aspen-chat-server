-- Every community without an owner is given to its earliest member whose account still
-- exists, usually whoever made it. A community with no such member keeps no owner.
UPDATE community c
SET owner = (
    SELECT cu."user"
    FROM community_user cu
    JOIN "user" u ON u.id = cu."user"
    WHERE cu.community = c.id AND u.deleted_at IS NULL
    ORDER BY cu.joined_at, cu."user"
    LIMIT 1
)
WHERE c.owner IS NULL;
