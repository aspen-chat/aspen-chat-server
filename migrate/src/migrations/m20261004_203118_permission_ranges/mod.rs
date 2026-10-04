use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261004_203118_permission_ranges",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
