use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261002_142250_community_ban",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
