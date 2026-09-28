use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_041109_community_user_by_community",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
