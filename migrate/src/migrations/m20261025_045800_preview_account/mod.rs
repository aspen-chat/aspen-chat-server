use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261025_045800_preview_account",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
