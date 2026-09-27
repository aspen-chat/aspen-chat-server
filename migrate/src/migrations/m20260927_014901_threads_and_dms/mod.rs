use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260927_014901_threads_and_dms",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
