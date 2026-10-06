use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261006_154221_voice_server_suspension",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
