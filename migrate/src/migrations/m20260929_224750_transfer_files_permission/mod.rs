use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260929_224750_transfer_files_permission",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
