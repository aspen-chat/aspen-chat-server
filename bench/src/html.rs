//! The report as a page (one self-contained HTML file with inline SVG charts, readable
//! offline and attachable anywhere) and as a few lines for the terminal.

use crate::report::{Point, Report};
use crate::scrape::Sample;
use aspen_bench_protocol::coordination::Phase;
use std::collections::BTreeMap;
use std::fmt::Write;

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn ms(value: f64) -> String {
    if value >= 100.0 {
        format!("{value:.0} ms")
    } else if value >= 10.0 {
        format!("{value:.1} ms")
    } else {
        format!("{value:.2} ms")
    }
}

/// The verdict and headline figures, for the terminal.
pub fn summary(report: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}: {} with {} users online",
        report.profile,
        if report.verdict.pass { "PASS" } else { "FAIL" },
        report.online_users
    );
    for check in &report.verdict.checks {
        let _ = writeln!(
            out,
            "  {} {:<40} target {:>10.3}  actual {}",
            if check.pass { "ok  " } else { "FAIL" },
            check.name,
            check.target,
            check.actual.map_or("none".into(), |a| format!("{a:.3}"))
        );
    }
    if !report.verdict.generator_kept_up {
        let _ = writeln!(
            out,
            "  the load generator fell behind (lag p99 {}); add agents and run again",
            ms(report.verdict.generator_lag_p99_ms)
        );
    }
    if let Some(steady) = report.steady() {
        let _ = writeln!(
            out,
            "  {:.0} requests/s, {:.1} messages/s, {:.0} events/s delivered",
            steady.per_second("requests"),
            steady.per_second("messages_sent"),
            steady.per_second("events")
        );
    }
    if let Some(step) = report.capacity.iter().rev().find(|s| s.pass) {
        let _ = writeln!(
            out,
            "  capacity: held {} users online ({} connected)",
            step.online_users, step.connected
        );
    }
    if let Some(first) = report.findings.first() {
        let _ = writeln!(
            out,
            "  first to run short: {} at {} ({})",
            first.what, first.endpoint, first.detail
        );
    }
    out
}

struct Series {
    label: String,
    points: Vec<(f64, f64)>,
}

const COLOURS: [&str; 6] = [
    "var(--a)", "var(--b)", "var(--c)", "var(--d)", "var(--e)", "var(--f)",
];

/// A line chart, as inline SVG.
fn chart(title: &str, unit: &str, series: &[Series]) -> String {
    let (w, h, left, bottom, top, right) = (640.0, 220.0, 56.0, 28.0, 12.0, 12.0);
    let all: Vec<(f64, f64)> = series
        .iter()
        .flat_map(|s| s.points.iter().copied())
        .collect();
    if all.is_empty() {
        return String::new();
    }
    let x_max = all.iter().map(|p| p.0).fold(1.0f64, f64::max);
    let x_min = all
        .iter()
        .map(|p| p.0)
        .fold(f64::INFINITY, f64::min)
        .min(x_max - 1.0);
    let peak = all.iter().map(|p| p.1).fold(0.0f64, f64::max);
    // A flat line at zero still gets a readable axis.
    let y_max = if peak > 0.0 { peak * 1.1 } else { 1.0 };
    let x = |v: f64| left + (v - x_min) / (x_max - x_min) * (w - left - right);
    let y = |v: f64| h - bottom - v / y_max * (h - bottom - top);
    let mut svg = format!(
        r#"<figure><figcaption>{}</figcaption><svg viewBox="0 0 {w} {h}" role="img" aria-label="{}">"#,
        escape(title),
        escape(title)
    );
    for i in 0..=4 {
        let v = y_max * f64::from(i) / 4.0;
        let _ = write!(
            svg,
            r#"<line x1="{left}" x2="{}" y1="{y:.1}" y2="{y:.1}" class="grid"/><text x="{}" y="{:.1}" class="tick" text-anchor="end">{}</text>"#,
            w - right,
            left - 6.0,
            y(v) + 4.0,
            format_value(v, unit),
            y = y(v)
        );
    }
    for i in 0..=5 {
        let v = x_min + (x_max - x_min) * f64::from(i) / 5.0;
        let _ = write!(
            svg,
            r#"<text x="{:.1}" y="{}" class="tick" text-anchor="middle">{:.0}s</text>"#,
            x(v),
            h - 8.0,
            v
        );
    }
    for (i, s) in series.iter().enumerate() {
        let path: Vec<String> = s
            .points
            .iter()
            .map(|(px, py)| format!("{:.1},{:.1}", x(*px), y(*py)))
            .collect();
        let _ = write!(
            svg,
            r#"<polyline points="{}" fill="none" stroke="{}" stroke-width="2"/>"#,
            path.join(" "),
            COLOURS[i % COLOURS.len()]
        );
    }
    svg.push_str("</svg>");
    if series.len() > 1 {
        svg.push_str("<div class=\"legend\">");
        for (i, s) in series.iter().enumerate() {
            let _ = write!(
                svg,
                r#"<span><i style="background:{}"></i>{}</span>"#,
                COLOURS[i % COLOURS.len()],
                escape(&s.label)
            );
        }
        svg.push_str("</div>");
    }
    svg.push_str("</figure>");
    svg
}

fn format_value(v: f64, unit: &str) -> String {
    let n = if v >= 10_000.0 {
        format!("{:.0}k", v / 1000.0)
    } else if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 1.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    };
    format!("{n}{unit}")
}

fn from_timeline(timeline: &[Point], label: &str, value: impl Fn(&Point) -> Option<f64>) -> Series {
    Series {
        label: label.to_string(),
        points: timeline
            .iter()
            .filter_map(|p| value(p).map(|v| (p.t, v)))
            .collect(),
    }
}

/// A counter's rate per endpoint over time.
fn server_rates(samples: &[Sample], series: &str) -> Vec<Series> {
    let mut by_endpoint: BTreeMap<&str, Vec<&Sample>> = BTreeMap::new();
    for s in samples {
        by_endpoint.entry(&s.endpoint).or_default().push(s);
    }
    by_endpoint
        .into_iter()
        .filter_map(|(endpoint, list)| {
            let points: Vec<(f64, f64)> = list
                .windows(2)
                .filter_map(|w| {
                    let dt = w[1].t - w[0].t;
                    let dv = w[1].values.get(series)? - w[0].values.get(series)?;
                    (dt > 0.0).then(|| (w[1].t, dv / dt))
                })
                .collect();
            (!points.is_empty()).then(|| Series {
                label: endpoint.to_string(),
                points,
            })
        })
        .collect()
}

fn server_gauge(samples: &[Sample], series: &str) -> Vec<Series> {
    let mut by_endpoint: BTreeMap<&str, Vec<(f64, f64)>> = BTreeMap::new();
    for s in samples {
        if let Some(v) = s.values.get(series) {
            by_endpoint.entry(&s.endpoint).or_default().push((s.t, *v));
        }
    }
    by_endpoint
        .into_iter()
        .map(|(endpoint, points)| Series {
            label: endpoint.to_string(),
            points,
        })
        .collect()
}

pub fn render(report: &Report) -> String {
    let mut body = String::new();
    let verdict = if report.verdict.pass {
        "Held up"
    } else {
        "Did not hold up"
    };
    let _ = write!(
        body,
        r#"<header><h1>{}</h1><p class="verdict {}">{verdict}</p><p>{} users online over {} agent{}, run <code>{}</code>.</p><p class="muted">{}</p></header>"#,
        escape(&report.profile),
        if report.verdict.pass { "pass" } else { "fail" },
        report.online_users,
        report.agents,
        if report.agents == 1 { "" } else { "s" },
        escape(&report.run),
        escape(&report.description)
    );
    if !report.verdict.generator_kept_up {
        let _ = write!(
            body,
            r#"<p class="warning">The load generator fell behind its own schedule (lag p99 {}), so these figures describe it as much as the deployment. Add agents and run again.</p>"#,
            ms(report.verdict.generator_lag_p99_ms)
        );
    }

    body.push_str("<section><h2>Service levels</h2><table><thead><tr><th>Level</th><th>Target</th><th>Measured</th><th></th></tr></thead><tbody>");
    for check in &report.verdict.checks {
        let _ = write!(
            body,
            "<tr><td>{}</td><td>{:.3}</td><td>{}</td><td class=\"{}\">{}</td></tr>",
            escape(&check.name),
            check.target,
            check.actual.map_or("none".into(), |a| format!("{a:.3}")),
            if check.pass { "pass" } else { "fail" },
            if check.pass { "met" } else { "missed" }
        );
    }
    body.push_str("</tbody></table></section>");

    if let Some(steady) = report.steady() {
        let figure = |name: &str| steady.metrics.get(name);
        let _ = write!(
            body,
            r#"<section><h2>Steady phase</h2><div class="figures"><div><b>{:.0}</b>requests/s</div><div><b>{:.1}</b>messages/s</div><div><b>{:.0}</b>events/s delivered</div><div><b>{}</b>delivery p50</div><div><b>{}</b>delivery p99</div><div><b>{}</b>request p99</div></div>"#,
            steady.per_second("requests"),
            steady.per_second("messages_sent"),
            steady.per_second("events"),
            figure("delivery").map_or("—".into(), |m| ms(m.p50_ms)),
            figure("delivery").map_or("—".into(), |m| ms(m.p99_ms)),
            figure("http:*").map_or("—".into(), |m| ms(m.p99_ms)),
        );
        body.push_str("<table><thead><tr><th>Measurement</th><th>Count</th><th>p50</th><th>p90</th><th>p99</th><th>p99.9</th><th>Max</th></tr></thead><tbody>");
        for (name, m) in &steady.metrics {
            let _ = write!(
                body,
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(name),
                m.count,
                ms(m.p50_ms),
                ms(m.p90_ms),
                ms(m.p99_ms),
                ms(m.p999_ms),
                ms(m.max_ms)
            );
        }
        body.push_str("</tbody></table>");
        let errors: Vec<(&String, &u64)> = steady
            .counters
            .iter()
            .filter(|(k, _)| k.starts_with("status:") && !k.contains(":2"))
            .collect();
        if !errors.is_empty() {
            body.push_str("<h3>Failed requests</h3><table><tbody>");
            for (name, n) in errors {
                let _ = write!(
                    body,
                    "<tr><td>{}</td><td>{n}</td></tr>",
                    escape(name.trim_start_matches("status:"))
                );
            }
            body.push_str("</tbody></table>");
        }
        body.push_str("</section>");
    }

    let interval = report
        .timeline
        .windows(2)
        .map(|w| w[1].t - w[0].t)
        .find(|d| *d > 0.0)
        .unwrap_or(5.0);
    body.push_str("<section><h2>Over time</h2>");
    body.push_str(&chart(
        "Users connected",
        "",
        &[from_timeline(&report.timeline, "connected", |p| {
            Some(p.connected as f64)
        })],
    ));
    body.push_str(&chart(
        "Throughput",
        "/s",
        &[
            from_timeline(&report.timeline, "requests", |p| {
                Some(p.counters.get("requests").copied().unwrap_or(0) as f64 / interval)
            }),
            from_timeline(&report.timeline, "events delivered", |p| {
                Some(p.counters.get("events").copied().unwrap_or(0) as f64 / interval)
            }),
        ],
    ));
    body.push_str(&chart(
        "Latency p99",
        " ms",
        &[
            from_timeline(&report.timeline, "delivery", |p| {
                p.metrics.get("delivery").map(|m| m.p99_ms)
            }),
            from_timeline(&report.timeline, "requests", |p| {
                p.metrics
                    .iter()
                    .filter(|(k, _)| {
                        k.starts_with("http:") && k.as_str() != "http:POST /auth/login"
                    })
                    .map(|(_, m)| m.p99_ms)
                    .reduce(f64::max)
            }),
            from_timeline(&report.timeline, "voice jitter", |p| {
                p.metrics.get("voice:jitter").map(|m| m.p99_ms)
            }),
        ],
    ));
    body.push_str(&chart(
        "Failures",
        "/s",
        &[
            from_timeline(&report.timeline, "errors", |p| {
                Some(p.counters.get("errors").copied().unwrap_or(0) as f64 / interval)
            }),
            from_timeline(&report.timeline, "refused as too fast", |p| {
                Some(p.counters.get("rate_limited").copied().unwrap_or(0) as f64 / interval)
            }),
            from_timeline(&report.timeline, "streams lost", |p| {
                Some(p.counters.get("disconnects").copied().unwrap_or(0) as f64 / interval)
            }),
        ],
    ));
    body.push_str("</section>");

    if !report.server_samples.is_empty() {
        body.push_str("<section><h2>The deployment</h2>");
        if report.findings.is_empty() {
            body.push_str("<p>No resource showed signs of running short.</p>");
        } else {
            body.push_str("<p>Resources that ran short, in the order they did. The first is the likeliest bottleneck.</p><table><thead><tr><th>At</th><th>Where</th><th>What</th><th>Detail</th></tr></thead><tbody>");
            for f in &report.findings {
                let _ = write!(
                    body,
                    "<tr><td>{:.0}s</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    f.t,
                    escape(&f.endpoint),
                    escape(&f.what),
                    escape(&f.detail)
                );
            }
            body.push_str("</tbody></table>");
        }
        body.push_str(&chart(
            "Process CPU (seconds per second)",
            "",
            &server_rates(&report.server_samples, "process_cpu_seconds_total"),
        ));
        body.push_str(&chart(
            "Resident memory",
            " B",
            &server_gauge(&report.server_samples, "process_resident_memory_bytes"),
        ));
        body.push_str(&chart(
            "Heap allocated (what the program holds)",
            " B",
            &server_gauge(&report.server_samples, aspen_metrics::memory::ALLOCATED),
        ));
        body.push_str(&chart(
            "Requests waiting for a database connection",
            "",
            &server_gauge(
                &report.server_samples,
                &format!("{}{{state=\"waiting\"}}", aspen_metrics::api::DB_POOL),
            ),
        ));
        body.push_str(&chart(
            "Open event streams",
            "",
            &server_gauge(&report.server_samples, aspen_metrics::api::EVENT_STREAMS),
        ));
        body.push_str(&chart(
            "Voice participants",
            "",
            &server_gauge(&report.server_samples, aspen_metrics::voice::PARTICIPANTS),
        ));
        if !report.memory_growth_per_hour.is_empty() {
            body.push_str("<h3>Memory growth per hour</h3><p>Resident memory that grows while the heap does not is the allocator keeping freed memory; a growing heap is a leak.</p><table><thead><tr><th>Server</th><th>Resident</th><th>Heap</th></tr></thead><tbody>");
            for (endpoint, growth) in &report.memory_growth_per_hour {
                let heap = report
                    .heap_growth_per_hour
                    .get(endpoint)
                    .map_or("—".to_string(), |g| format!("{:+.1} MB", g / 1e6));
                let _ = write!(
                    body,
                    "<tr><td>{}</td><td>{:+.1} MB</td><td>{heap}</td></tr>",
                    escape(endpoint),
                    growth / 1e6
                );
            }
            body.push_str("</tbody></table>");
        }
        body.push_str("</section>");
    }

    if !report.capacity.is_empty() {
        body.push_str("<section><h2>Capacity</h2><table><thead><tr><th>Users online</th><th>Connected</th><th>Held</th><th>Delivery p99</th><th>Request p99</th><th>First to run short</th></tr></thead><tbody>");
        for step in &report.capacity {
            let m = |name: &str| {
                step.steady
                    .as_ref()
                    .and_then(|s| s.metrics.get(name))
                    .map_or("—".into(), |m| ms(m.p99_ms))
            };
            let _ = write!(
                body,
                "<tr><td>{}</td><td>{}</td><td class=\"{}\">{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                step.online_users,
                step.connected,
                if step.pass { "pass" } else { "fail" },
                if step.pass { "yes" } else { "no" },
                m("delivery"),
                m("http:*"),
                step.findings
                    .first()
                    .map_or("—".into(), |f| escape(&format!(
                        "{} at {}",
                        f.what, f.endpoint
                    )))
            );
        }
        body.push_str("</tbody></table></section>");
    }

    if !report.commands.is_empty() {
        body.push_str("<section><h2>Commands run</h2><table><tbody>");
        for c in &report.commands {
            let _ = write!(
                body,
                "<tr><td>{:.0}s</td><td><code>{}</code></td><td>{}</td></tr>",
                c.t,
                escape(&c.command),
                c.status
                    .map_or("did not start".into(), |s| format!("exit {s}"))
            );
        }
        body.push_str("</tbody></table></section>");
    }

    let phases: Vec<String> = report
        .phases
        .iter()
        .map(|(phase, stats)| {
            format!(
                "{} {:.0}s",
                match phase {
                    Phase::Ramp => "ramp",
                    Phase::Steady => "steady",
                    Phase::Drain => "drain",
                },
                stats.seconds
            )
        })
        .collect();
    let _ = write!(
        body,
        r#"<footer class="muted">Phases: {}. Generated by aspen-bench.</footer>"#,
        phases.join(", ")
    );

    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{} benchmark</title><style>{STYLE}</style></head><body><main>{body}</main></body></html>"#,
        escape(&report.profile)
    )
}

const STYLE: &str = r#"
:root { color-scheme: light dark; --bg: #f7f7f5; --panel: #fff; --ink: #1d1d1b; --muted: #66665f; --line: #dcdcd6; --pass: #1f7a4d; --fail: #b3261e; --a: #2f6f9f; --b: #c26a1a; --c: #6a4fa3; --d: #1f7a4d; --e: #a3294f; --f: #587a1f; }
@media (prefers-color-scheme: dark) { :root { --bg: #161615; --panel: #20201e; --ink: #ececea; --muted: #a3a39d; --line: #34342f; --pass: #5fbf8a; --fail: #ef8a80; --a: #7fb2dd; --b: #f0a35c; --c: #b39ae6; --d: #5fbf8a; --e: #ea7aa0; --f: #b5d16a; } }
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--ink); font: 15px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }
main { max-width: 980px; margin: 0 auto; padding: 24px 16px 48px; }
header, section { background: var(--panel); border: 1px solid var(--line); border-radius: 12px; padding: 20px; margin-bottom: 16px; }
h1 { margin: 0 0 4px; font-size: 1.5rem; } h2 { margin: 0 0 12px; font-size: 1.15rem; } h3 { font-size: 1rem; margin: 16px 0 8px; }
.verdict { font-size: 1.25rem; font-weight: 700; margin: 4px 0; } .verdict.pass, td.pass { color: var(--pass); } .verdict.fail, td.fail { color: var(--fail); }
.muted { color: var(--muted); } .warning { border-left: 4px solid var(--fail); padding: 8px 12px; background: var(--panel); }
table { border-collapse: collapse; width: 100%; font-variant-numeric: tabular-nums; } th, td { text-align: left; padding: 6px 8px; border-bottom: 1px solid var(--line); } th { color: var(--muted); font-weight: 600; }
.figures { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 12px; margin-bottom: 16px; } .figures div { border: 1px solid var(--line); border-radius: 8px; padding: 10px; color: var(--muted); } .figures b { display: block; font-size: 1.3rem; color: var(--ink); }
figure { margin: 0 0 20px; } figcaption { font-weight: 600; margin-bottom: 4px; } svg { width: 100%; height: auto; }
.grid { stroke: var(--line); } .tick { fill: var(--muted); font-size: 11px; }
.legend { display: flex; flex-wrap: wrap; gap: 12px; color: var(--muted); font-size: 13px; } .legend i { display: inline-block; width: 12px; height: 3px; margin-right: 6px; vertical-align: middle; }
code { font-size: 0.9em; } footer { font-size: 13px; margin-top: 8px; }
@media (max-width: 600px) { th, td { padding: 4px; font-size: 13px; } }
"#;
