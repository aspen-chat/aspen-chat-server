use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261008_153045_query_growth",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
