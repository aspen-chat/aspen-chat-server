use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261002_062747_custom_emoji",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
