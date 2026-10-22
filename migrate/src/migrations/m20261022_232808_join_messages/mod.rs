use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261022_232808_join_messages",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
