use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_020453_channel_mute",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
