use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261124_055144_svg_previews",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
