use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261008_170000_jobs",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
