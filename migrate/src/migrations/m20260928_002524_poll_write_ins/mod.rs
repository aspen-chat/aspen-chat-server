use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_002524_poll_write_ins",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
