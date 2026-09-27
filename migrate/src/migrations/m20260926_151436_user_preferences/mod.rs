use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260926_151436_user_preferences",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
