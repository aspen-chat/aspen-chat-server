use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260930_081607_link_preview_image_size",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
