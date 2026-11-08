use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261105_143743_scheduled_messages",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
