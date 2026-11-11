use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261111_143412_moderation_log_operator",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
