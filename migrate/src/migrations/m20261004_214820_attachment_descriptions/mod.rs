use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261004_214820_attachment_descriptions",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
