use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261023_120000_attachment_spoilers",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
