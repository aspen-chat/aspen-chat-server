use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261018_224440_channel_mana",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
