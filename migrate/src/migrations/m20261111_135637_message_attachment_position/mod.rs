use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261111_135637_message_attachment_position",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
