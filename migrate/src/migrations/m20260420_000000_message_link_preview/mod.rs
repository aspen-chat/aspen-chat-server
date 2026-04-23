use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260420_000000_message_link_preview",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
