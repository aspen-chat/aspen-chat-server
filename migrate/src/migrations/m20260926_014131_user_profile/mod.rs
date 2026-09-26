use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260926_014131_user_profile",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
