use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261009_072442_held_message_channel_index",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
