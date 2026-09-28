use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_070500_community_owners",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
