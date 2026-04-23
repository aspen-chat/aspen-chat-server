use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260317_053302_message_timestamp",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
