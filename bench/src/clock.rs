//! The coordinator's clock, as an agent reads it: its own wall clock plus a measured offset.
//! Every timestamp that crosses machines (a message's send time, the run's start) is on it.

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    offset_ns: i64,
}

impl Clock {
    pub fn new(offset_ns: i64) -> Self {
        Self { offset_ns }
    }

    /// This machine's wall clock, in Unix nanoseconds.
    pub fn local_ns() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as i64)
    }

    /// The coordinator's clock now.
    pub fn now_ns(&self) -> i64 {
        Self::local_ns() + self.offset_ns
    }

    /// How long from now until coordinator time `at_ns`, zero if it has passed.
    pub fn until(&self, at_ns: i64) -> std::time::Duration {
        let wait = at_ns - self.now_ns();
        std::time::Duration::from_nanos(u64::try_from(wait.max(0)).unwrap_or(u64::MAX))
    }
}

/// The offset of the coordinator's clock from an agent's, from ping rounds `(sent, coordinator,
/// received)`: the round with the shortest trip bounds the error by half that trip, so its
/// midpoint is taken (Cristian's algorithm).
pub fn estimate_offset(rounds: &[(i64, i64, i64)]) -> Option<(i64, i64)> {
    rounds
        .iter()
        .min_by_key(|(sent, _, received)| received - sent)
        .map(|(sent, coordinator, received)| {
            let trip = received - sent;
            (coordinator - (sent + trip / 2), trip / 2)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortest_round_sets_the_offset() {
        // The coordinator is 1000 ahead. Round one had a slow return leg; round two was quick.
        let rounds = [(0, 1100, 400), (500, 1520, 540)];
        let (offset, error) = estimate_offset(&rounds).unwrap();
        assert_eq!(offset, 1000);
        assert_eq!(error, 20);
        assert!(estimate_offset(&[]).is_none());
    }
}
