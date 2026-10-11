use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261011_053939_mention_recent",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
