use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261003_070819_reports",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
