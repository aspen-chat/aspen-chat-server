use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261004_062257_role_colors",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
