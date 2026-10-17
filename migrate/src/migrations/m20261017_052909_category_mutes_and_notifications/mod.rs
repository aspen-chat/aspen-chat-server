use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261017_052909_category_mutes_and_notifications",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
