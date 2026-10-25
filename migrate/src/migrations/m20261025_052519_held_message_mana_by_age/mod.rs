use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261025_052519_held_message_mana_by_age",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
