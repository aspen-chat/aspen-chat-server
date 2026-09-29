use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260929_063942_notification_setting",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
