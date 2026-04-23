use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260409_000001_soft_deletes",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
