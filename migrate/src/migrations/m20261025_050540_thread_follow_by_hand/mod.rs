use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261025_050540_thread_follow_by_hand",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
