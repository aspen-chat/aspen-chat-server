use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261004_100025_nicknames",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
