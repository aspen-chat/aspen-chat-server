use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261003_090734_icon_uploader",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
