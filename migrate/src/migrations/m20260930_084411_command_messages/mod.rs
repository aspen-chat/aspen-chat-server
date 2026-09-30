use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260930_084411_command_messages",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
