use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260929_001328_federation_peer_protocol",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
