//! Audio levels: what decides speaking and fills the audio slots.
//!
//! Each audio packet carries its level (RFC 6464, `0` loudest to `127` silence, in -dBov). Over
//! each `INTERVAL` a source's levels are averaged, as mediasoup's audio level observer does; a
//! source whose average is above `SPEAKING_THRESHOLD_DBOV` is speaking.

use std::time::{Duration, Instant};

pub const INTERVAL: Duration = Duration::from_millis(300);
/// Louder than this, on average over an interval, is speaking.
pub const SPEAKING_THRESHOLD_DBOV: i32 = -50;

/// One source's levels in the current interval, and when it last spoke.
#[derive(Debug, Default, Clone)]
pub struct Levels {
    /// Sum of the interval's levels, in dBov (zero or less).
    sum: i64,
    count: u32,
    speaking: bool,
    pub last_spoke: Option<Instant>,
}

impl Levels {
    /// Takes one packet's RFC 6464 level, `-127..=0` dBov as str0m gives it.
    pub fn hear(&mut self, level: i8) {
        self.sum += i64::from(level.min(0));
        self.count += 1;
    }

    /// Ends the interval: whether the source is speaking now, and whether that changed. A
    /// source that sent no level this interval is silent.
    pub fn close_interval(&mut self, now: Instant) -> Option<bool> {
        let average = (self.count > 0).then(|| self.sum / i64::from(self.count));
        let speaking = average.is_some_and(|dbov| dbov > i64::from(SPEAKING_THRESHOLD_DBOV));
        self.sum = 0;
        self.count = 0;
        if speaking {
            self.last_spoke = Some(now);
        }
        (speaking != self.speaking).then(|| {
            self.speaking = speaking;
            speaking
        })
    }

    /// Silences the source at once, as a mute does, rather than at the interval's end; whether
    /// that changed anything.
    pub fn silence(&mut self) -> bool {
        self.sum = 0;
        self.count = 0;
        std::mem::replace(&mut self.speaking, false)
    }

    pub fn speaking(&self) -> bool {
        self.speaking
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaking_is_an_interval_louder_than_the_threshold_on_average() {
        let now = Instant::now();
        let mut levels = Levels::default();
        for _ in 0..10 {
            levels.hear(-30);
        }
        assert_eq!(levels.close_interval(now), Some(true));
        assert_eq!(levels.last_spoke, Some(now));
        // Still speaking: no change to tell.
        levels.hear(-40);
        assert_eq!(levels.close_interval(now), None);
        // Quieter than the threshold on average, though one packet was loud.
        levels.hear(-20);
        levels.hear(-90);
        levels.hear(-90);
        assert_eq!(levels.close_interval(now), Some(false));
        // An interval with nothing heard is silence.
        assert_eq!(levels.close_interval(now), None);
    }

    #[test]
    fn silencing_ends_speaking_at_once() {
        let mut levels = Levels::default();
        levels.hear(-10);
        levels.close_interval(Instant::now());
        assert!(levels.silence());
        assert!(!levels.speaking());
        assert!(!levels.silence());
    }
}
