use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261009_094225_presence_override",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
