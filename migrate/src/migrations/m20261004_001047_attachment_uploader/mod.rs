use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261004_001047_attachment_uploader",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
