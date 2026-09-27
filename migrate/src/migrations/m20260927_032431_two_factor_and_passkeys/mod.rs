use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260927_032431_two_factor_and_passkeys",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
