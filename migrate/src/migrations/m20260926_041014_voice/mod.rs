use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260926_041014_voice",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
