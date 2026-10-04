use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261003_233932_deleted_category_leftovers",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
