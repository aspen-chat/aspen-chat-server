use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260930_075132_attachment_dimensions",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
