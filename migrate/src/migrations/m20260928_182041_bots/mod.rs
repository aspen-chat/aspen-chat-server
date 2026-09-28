use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_182041_bots",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
