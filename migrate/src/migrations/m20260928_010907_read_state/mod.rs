use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_010907_read_state",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
