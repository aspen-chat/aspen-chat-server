use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260929_233735_file_transfer_log",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
