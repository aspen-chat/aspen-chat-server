use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260930_044153_use_camera_permission",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
