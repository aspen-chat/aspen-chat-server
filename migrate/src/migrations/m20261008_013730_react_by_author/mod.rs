use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261008_013730_react_by_author",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
