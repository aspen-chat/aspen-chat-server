use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261006_154118_recovery_code_created_at",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
