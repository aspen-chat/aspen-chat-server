use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261007_120000_server_secret",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
