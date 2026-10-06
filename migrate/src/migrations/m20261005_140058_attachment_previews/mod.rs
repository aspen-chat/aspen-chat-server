use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261005_140058_attachment_previews",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
