use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261006_150944_token_digests",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
