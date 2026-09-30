use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260930_031756_missed_calls",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
