use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260317_065719_message_attachments",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
