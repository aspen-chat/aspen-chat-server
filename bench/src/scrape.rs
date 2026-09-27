//! Sampling the deployment's Prometheus endpoints during a run, and reading what they say about
//! where it ran out.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// One sample of one endpoint: every series by name with its labels, as Prometheus writes it
/// (`aspen_db_pool_connections{state="waiting"}`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    /// Seconds from the run's start.
    pub t: f64,
    pub endpoint: String,
    pub values: BTreeMap<String, f64>,
}

/// The series worth keeping: Aspen's own, the processes', and the usual exporters' headline
/// figures.
fn keep(series: &str) -> bool {
    series.starts_with("aspen_")
        || series.starts_with("process_cpu_seconds_total")
        || series.starts_with("process_resident_memory_bytes")
        || series.starts_with("process_open_fds")
        || series.starts_with("pg_stat_database_numbackends")
        || series.starts_with("pg_stat_activity_count")
        || series.starts_with("nats_")
        || series.starts_with("redis_connected_clients")
        || series.starts_with("redis_cpu_")
        || series.starts_with("node_load1")
}

/// Parses the Prometheus text format, keeping `keep`'s series. Histogram buckets are kept too,
/// so percentiles can be read from them.
pub fn parse(text: &str) -> BTreeMap<String, f64> {
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .filter_map(|line| {
            // The value follows the last space outside the labels.
            let split = match line.rfind('}') {
                Some(close) => close + 1 + line[close + 1..].find(' ')?,
                None => line.find(' ')?,
            };
            let (series, rest) = line.split_at(split);
            let value = rest.split_whitespace().next()?.parse::<f64>().ok()?;
            keep(series).then(|| (series.to_string(), value))
        })
        .collect()
}

pub async fn sample(client: &reqwest::Client, endpoint: &str, t: f64) -> Option<Sample> {
    let text = client
        .get(endpoint)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    Some(Sample {
        t,
        endpoint: endpoint.to_string(),
        values: parse(&text),
    })
}

/// A resource that ran short, and when it first did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Seconds from the run's start.
    pub t: f64,
    pub endpoint: String,
    pub what: String,
    pub detail: String,
}

fn value(sample: &Sample, series: &str) -> Option<f64> {
    sample.values.get(series).copied()
}

/// The rate of a counter between two samples of one endpoint.
fn rate(before: &Sample, after: &Sample, series: &str) -> Option<f64> {
    let dt = after.t - before.t;
    if dt <= 0.0 {
        return None;
    }
    Some((value(after, series)? - value(before, series)?) / dt)
}

/// A histogram's quantile between two samples, from its cumulative buckets.
fn quantile(before: &Sample, after: &Sample, metric: &str, q: f64) -> Option<f64> {
    let prefix = format!("{metric}_bucket{{");
    let mut buckets: Vec<(f64, f64)> = after
        .values
        .iter()
        .filter(|(k, _)| k.starts_with(&prefix))
        .filter_map(|(k, v)| {
            let le = k.split("le=\"").nth(1)?.split('"').next()?;
            let le = if le == "+Inf" {
                f64::INFINITY
            } else {
                le.parse().ok()?
            };
            let earlier = before.values.get(k).copied().unwrap_or(0.0);
            Some((le, v - earlier))
        })
        .collect();
    buckets.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total = buckets.last()?.1;
    if total <= 0.0 {
        return None;
    }
    buckets
        .iter()
        .find(|(_, count)| *count >= total * q)
        .map(|(le, _)| *le)
}

/// Reads the samples for signs of a resource running short, each reported the first time it
/// shows. Findings come in time order; the first is the likeliest bottleneck.
pub fn findings(samples: &[Sample]) -> Vec<Finding> {
    let mut by_endpoint: BTreeMap<&str, Vec<&Sample>> = BTreeMap::new();
    for s in samples {
        by_endpoint.entry(s.endpoint.as_str()).or_default().push(s);
    }
    let mut found: Vec<Finding> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut note = |t: f64, endpoint: &str, what: &str, detail: String| {
        if seen.insert((endpoint.to_string(), what.to_string())) {
            found.push(Finding {
                t,
                endpoint: endpoint.to_string(),
                what: what.to_string(),
                detail,
            });
        }
    };
    for (endpoint, series) in by_endpoint {
        for pair in series.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let cpus = value(b, aspen_metrics::HOST_CPUS).unwrap_or(1.0);
            if let Some(cpu) = rate(a, b, "process_cpu_seconds_total")
                && cpu >= 0.85 * cpus
            {
                note(
                    b.t,
                    endpoint,
                    "process CPU",
                    format!("{cpu:.2} of {cpus} CPUs busy"),
                );
            }
            // Sustained: a burst of reconnections to a freshly started server briefly queues
            // for connections the pool has not opened yet.
            let waiting_series = format!("{}{{state=\"waiting\"}}", aspen_metrics::api::DB_POOL);
            if let (Some(before), Some(waiting)) =
                (value(a, &waiting_series), value(b, &waiting_series))
                && before > 0.0
                && waiting > 0.0
            {
                note(
                    b.t,
                    endpoint,
                    "database connections",
                    format!("{waiting} requests waiting for a connection"),
                );
            }
            if let Some(p99) = quantile(a, b, aspen_metrics::api::EVENT_PUBLISH_DURATION, 0.99)
                && p99 >= 0.05
            {
                note(
                    b.t,
                    endpoint,
                    "NATS JetStream",
                    format!(
                        "99% of event publishes acknowledged within {:.0} ms",
                        p99 * 1000.0
                    ),
                );
            }
            if let Some(p99) = quantile(a, b, aspen_metrics::api::RATE_LIMIT_CHECK_DURATION, 0.99)
                && p99 >= 0.02
            {
                note(
                    b.t,
                    endpoint,
                    "Valkey",
                    format!("99% of rate limit checks within {:.0} ms", p99 * 1000.0),
                );
            }
            let refused: f64 = b
                .values
                .iter()
                .filter(|(k, _)| {
                    k.starts_with(aspen_metrics::api::RATE_LIMIT_REFUSALS)
                        || k.starts_with(aspen_metrics::voice::FRAMES_REFUSED)
                        || k.starts_with(aspen_metrics::voice::HTTP_REFUSED)
                })
                .map(|(k, v)| v - a.values.get(k).copied().unwrap_or(0.0))
                .sum();
            if refused > 0.0 {
                note(
                    b.t,
                    endpoint,
                    "rate limits",
                    format!(
                        "{refused} requests refused as too fast; the limits, not the capacity, bound this run"
                    ),
                );
            }
            for (key, _) in b
                .values
                .iter()
                .filter(|(k, _)| k.starts_with(aspen_metrics::voice::WORKER_CPU))
            {
                if let Some(cpu) = rate(a, b, key)
                    && cpu >= 0.85
                {
                    note(
                        b.t,
                        endpoint,
                        "a mediasoup worker",
                        format!("{key} at {cpu:.2} of one CPU"),
                    );
                }
            }
        }
    }
    found.sort_by(|a, b| a.t.total_cmp(&b.t));
    found
}

/// Growth per hour of the gauge `series` at an endpoint over the samples, by least squares: a
/// steady climb of memory in a long run is a leak.
pub fn growth_per_hour(samples: &[Sample], endpoint: &str, series: &str) -> Option<f64> {
    let points: Vec<(f64, f64)> = samples
        .iter()
        .filter(|s| s.endpoint == endpoint)
        .filter_map(|s| Some((s.t, value(s, series)?)))
        .collect();
    if points.len() < 3 {
        return None;
    }
    let n = points.len() as f64;
    let (sx, sy) = points
        .iter()
        .fold((0.0, 0.0), |(x, y), p| (x + p.0, y + p.1));
    let (mx, my) = (sx / n, sy / n);
    let (num, den) = points.iter().fold((0.0, 0.0), |(num, den), (x, y)| {
        (num + (x - mx) * (y - my), den + (x - mx) * (x - mx))
    });
    (den > 0.0).then(|| num / den * 3600.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(t: f64, values: &[(&str, f64)]) -> Sample {
        Sample {
            t,
            endpoint: "api".into(),
            values: values.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    #[test]
    fn the_text_format_parses() {
        let text = "# HELP x y\n# TYPE aspen_event_streams gauge\naspen_event_streams 12\naspen_db_pool_connections{state=\"waiting\"} 3\nunrelated_series 5\naspen_http_request_duration_seconds_bucket{route=\"GET /a b\",le=\"0.5\"} 7\n";
        let parsed = parse(text);
        assert_eq!(parsed["aspen_event_streams"], 12.0);
        assert_eq!(parsed["aspen_db_pool_connections{state=\"waiting\"}"], 3.0);
        assert_eq!(
            parsed["aspen_http_request_duration_seconds_bucket{route=\"GET /a b\",le=\"0.5\"}"],
            7.0
        );
        assert!(!parsed.contains_key("unrelated_series"));
    }

    #[test]
    fn saturation_is_found_in_time_order() {
        let samples = vec![
            sample(
                0.0,
                &[("process_cpu_seconds_total", 0.0), ("aspen_host_cpus", 2.0)],
            ),
            sample(
                10.0,
                &[("process_cpu_seconds_total", 5.0), ("aspen_host_cpus", 2.0)],
            ),
            sample(
                20.0,
                &[
                    ("process_cpu_seconds_total", 23.0),
                    ("aspen_host_cpus", 2.0),
                    ("aspen_db_pool_connections{state=\"waiting\"}", 4.0),
                ],
            ),
            sample(
                30.0,
                &[
                    ("process_cpu_seconds_total", 25.0),
                    ("aspen_host_cpus", 2.0),
                    ("aspen_db_pool_connections{state=\"waiting\"}", 6.0),
                ],
            ),
        ];
        let found = findings(&samples);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].what, "process CPU");
        assert_eq!(found[0].t, 20.0);
        assert_eq!(found[1].what, "database connections");
        assert_eq!(found[1].t, 30.0, "one sample of waiting is not saturation");
    }

    #[test]
    fn histogram_quantiles_come_from_bucket_differences() {
        let bucket =
            |le: &str| format!("aspen_event_publish_duration_seconds_bucket{{le=\"{le}\"}}");
        let a = sample(
            0.0,
            &[
                (&bucket("0.01"), 100.0),
                (&bucket("0.1"), 100.0),
                (&bucket("+Inf"), 100.0),
            ],
        );
        let b = sample(
            10.0,
            &[
                (&bucket("0.01"), 110.0),
                (&bucket("0.1"), 200.0),
                (&bucket("+Inf"), 200.0),
            ],
        );
        assert_eq!(
            quantile(&a, &b, "aspen_event_publish_duration_seconds", 0.99),
            Some(0.1)
        );
        assert!(findings(&[a, b]).iter().any(|f| f.what == "NATS JetStream"));
    }

    #[test]
    fn memory_growth_is_a_slope() {
        let samples: Vec<Sample> = (0..5)
            .map(|i| {
                sample(
                    f64::from(i) * 60.0,
                    &[("process_resident_memory_bytes", 1e9 + f64::from(i) * 1e6)],
                )
            })
            .collect();
        let growth = growth_per_hour(&samples, "api", "process_resident_memory_bytes").unwrap();
        assert!((growth - 60e6).abs() < 1.0, "{growth}");
    }
}
