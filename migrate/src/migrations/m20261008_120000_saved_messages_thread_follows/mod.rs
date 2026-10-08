use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261008_120000_saved_messages_thread_follows",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
