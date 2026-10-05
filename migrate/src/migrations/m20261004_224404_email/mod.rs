use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261004_224404_email",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
