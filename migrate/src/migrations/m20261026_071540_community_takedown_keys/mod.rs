use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261026_071540_community_takedown_keys",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
