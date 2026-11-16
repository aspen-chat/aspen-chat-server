use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261116_101740_shrink_icons",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
