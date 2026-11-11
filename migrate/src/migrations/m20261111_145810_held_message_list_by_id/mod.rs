use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261111_145810_held_message_list_by_id",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
