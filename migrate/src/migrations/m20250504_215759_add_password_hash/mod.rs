use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20250504_215759_add_password_hash",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
