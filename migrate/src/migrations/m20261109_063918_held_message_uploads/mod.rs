use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261109_063918_held_message_uploads",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
