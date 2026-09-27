//! This server's metrics (`aspen_metrics::voice`): what it carries, sampled, and what its
//! clients ask of it, counted where it happens.

use crate::limits::Limits;
use crate::rooms::Rooms;
use std::sync::Arc;

/// Samples the census, the mediasoup workers' CPU time, and whether limits are suspended.
pub fn spawn_samplers(rooms: Arc<Rooms>, limits: Arc<Limits>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(aspen_metrics::SAMPLE_INTERVAL);
        loop {
            interval.tick().await;
            let census = rooms.census();
            for (name, value) in [
                (aspen_metrics::voice::ROOMS, census.rooms),
                (aspen_metrics::voice::PARTICIPANTS, census.participants),
                (aspen_metrics::voice::PRODUCERS, census.producers),
                (aspen_metrics::voice::CONSUMERS, census.consumers),
                (aspen_metrics::voice::TRANSPORTS, census.transports),
            ] {
                metrics::gauge!(name).set(value as f64);
            }
            for (worker, seconds) in worker_cpu_seconds() {
                metrics::gauge!(aspen_metrics::voice::WORKER_CPU, "worker" => worker).set(seconds);
            }
            let suspended = limits.suspension().current().is_some();
            metrics::gauge!(aspen_metrics::voice::RATE_LIMITS_SUSPENDED).set(if suspended {
                1.0
            } else {
                0.0
            });
        }
    });
}

/// CPU seconds (user and system) of each mediasoup worker. The workers are threads of this
/// process named `mediasoup-worker-{id}`, which Linux shortens to fifteen characters, so each is
/// labelled by its thread id. Empty where `/proc` is unavailable.
pub(crate) fn worker_cpu_seconds() -> Vec<(String, f64)> {
    // SAFETY: `sysconf` reads a constant of the running system.
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks_per_second <= 0 {
        return Vec::new();
    }
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return Vec::new();
    };
    tasks
        .filter_map(Result::ok)
        .filter_map(|task| {
            let path = task.path();
            let name = std::fs::read_to_string(path.join("comm")).ok()?;
            if !name.starts_with("mediasoup-worke") {
                return None;
            }
            let stat = std::fs::read_to_string(path.join("stat")).ok()?;
            let ticks = user_and_system_ticks(&stat)?;
            Some((
                task.file_name().to_string_lossy().into_owned(),
                ticks as f64 / ticks_per_second as f64,
            ))
        })
        .collect()
}

/// Fields 14 and 15 of `/proc/[pid]/task/[tid]/stat` (utime, stime). The second field is the
/// name in parentheses and may contain spaces, so counting starts after its closing `)`.
fn user_and_system_ticks(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 2..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // After the name, the state is field 3, so utime (14) and stime (15) are at 11 and 12.
    let user: u64 = fields.get(11)?.parse().ok()?;
    let system: u64 = fields.get(12)?.parse().ok()?;
    Some(user + system)
}

pub fn frame(kind: &'static str) {
    metrics::counter!(aspen_metrics::voice::FRAMES, "kind" => kind).increment(1);
}

pub fn frame_refused(kind: &'static str) {
    metrics::counter!(aspen_metrics::voice::FRAMES_REFUSED, "kind" => kind).increment(1);
}

pub fn http_refused(route: &'static str) {
    metrics::counter!(aspen_metrics::voice::HTTP_REFUSED, "route" => route).increment(1);
}

#[cfg(test)]
mod tests {
    use super::user_and_system_ticks;

    #[test]
    fn cpu_ticks_are_read_after_the_thread_name() {
        let stat = "1234 (mediasoup-worke) S 1 1234 1234 0 -1 4194560 100 0 0 0 250 75 0 0 20 0 1 0 100 0 0";
        assert_eq!(user_and_system_ticks(stat), Some(325));
        // A name with spaces and parentheses does not shift the fields.
        let odd = "1234 (a (b) c) S 1 1234 1234 0 -1 4194560 100 0 0 0 7 3 0 0 20 0 1 0 100 0 0";
        assert_eq!(user_and_system_ticks(odd), Some(10));
        assert_eq!(user_and_system_ticks("garbage"), None);
    }

    #[test]
    fn this_process_threads_can_be_read() {
        // No mediasoup workers run in tests, so nothing is found, but reading must not fail.
        let _ = super::worker_cpu_seconds();
    }
}
