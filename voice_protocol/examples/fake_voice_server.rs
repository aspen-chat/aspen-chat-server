//! A voice server with no media: it publishes the reports a real one would, so the API server's
//! side of the contract can be exercised without mediasoup. Each invocation sends one report,
//! to the report stream on its subject as the server it names, and waits for the stream to take
//! it.
//!
//! ```text
//! cargo run -p voice_protocol --example fake_voice_server -- \
//!     --nats nats://localhost:4222 --token <nats token> \
//!     session-started <server> <session> <channel>
//! ```
//!
//! Reports: `load <server> <participants>`, `session-started <server> <session> <channel>`,
//! `snapshot <server> <session> <channel> [<user>[:muted][:deafened][:sharing]...]`, `held
//! <server> <lane> [<session>...]`, and, sent as the server `--server <server>` names, `joined
//! <session> <channel> <user>`, `left <session> <channel> <user>`, `speaking <session> <channel>
//! <user> <true|false>`, `state <session> <channel> <user> <muted> <deafened>`, `ended <session>
//! <channel>`. Also `verify <token>
//! <secret> <server>` checks a join token the way a voice server would, and `command-kick
//! <server> <session> <user>` / `command-mute <server> <session> <user> <muted>` send a
//! command to a voice server the way the API server does.

use std::env;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use voice_protocol::control::{ParticipantSnapshot, VoiceCommand, VoiceReport, command_subject};
use voice_protocol::token::verify;

fn usage() -> ! {
    eprintln!(
        "usage: fake_voice_server --nats <url> --token <token> [--server <server>] <report> <args...>"
    );
    std::process::exit(2);
}

fn uuid(arg: Option<&String>) -> Uuid {
    arg.and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(|| usage())
}

fn flag(arg: Option<&String>) -> bool {
    matches!(arg.map(String::as_str), Some("true" | "1" | "yes"))
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut nats_url = "nats://localhost:4222".to_string();
    let mut nats_token = None;
    let mut sender = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--nats" => {
                nats_url = args.get(i + 1).cloned().unwrap_or_else(|| usage());
                i += 2;
            }
            "--token" => {
                nats_token = args.get(i + 1).cloned();
                i += 2;
            }
            "--server" => {
                sender = Some(uuid(args.get(i + 1)));
                i += 2;
            }
            _ => {
                rest.push(args[i].clone());
                i += 1;
            }
        }
    }
    let Some(command) = rest.first().map(String::as_str) else {
        usage();
    };
    if command == "verify" {
        let now = i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after the epoch")
                .as_secs(),
        )
        .expect("fits");
        let secret = rest.get(2).map(String::as_str).unwrap_or_else(|| usage());
        match verify(
            rest.get(1).map(String::as_str).unwrap_or_else(|| usage()),
            secret.as_bytes(),
            uuid(rest.get(3)),
            now,
        ) {
            Ok(claims) => println!("{}", serde_json::to_string(&claims).expect("serializes")),
            Err(e) => {
                eprintln!("refused: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let command_to_send = match command {
        "command-kick" => Some((
            uuid(rest.get(1)),
            VoiceCommand::Kick {
                session: uuid(rest.get(2)),
                user: uuid(rest.get(3)),
            },
        )),
        "command-mute" => Some((
            uuid(rest.get(1)),
            VoiceCommand::Mute {
                session: uuid(rest.get(2)),
                user: uuid(rest.get(3)),
                muted: flag(rest.get(4)),
            },
        )),
        _ => None,
    };
    if let Some((server, command)) = command_to_send {
        let mut options = async_nats::ConnectOptions::new();
        if let Some(token) = nats_token {
            options = options.token(token);
        }
        let client = async_nats::connect_with_options(&nats_url, options)
            .await
            .expect("connect to NATS");
        client
            .publish(
                command_subject(server),
                serde_json::to_vec(&command).expect("serializes").into(),
            )
            .await
            .expect("publish");
        client.flush().await.expect("flush");
        println!(
            "sent {}",
            serde_json::to_string(&command).expect("serializes")
        );
        return;
    }
    let report = match command {
        "snapshot" => VoiceReport::SessionSnapshot {
            server: uuid(rest.get(1)),
            session: uuid(rest.get(2)),
            channel: uuid(rest.get(3)),
            participants: rest
                .iter()
                .skip(4)
                .map(|participant| {
                    let mut parts = participant.split(':');
                    let user = uuid(parts.next().map(str::to_string).as_ref());
                    let marks: Vec<&str> = parts.collect();
                    ParticipantSnapshot {
                        user,
                        muted: marks.contains(&"muted"),
                        deafened: marks.contains(&"deafened"),
                        sharing_screen: marks.contains(&"sharing"),
                    }
                })
                .collect(),
        },
        "held" => VoiceReport::SessionsHeld {
            server: uuid(rest.get(1)),
            partition: rest
                .get(2)
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| usage()),
            sessions: rest.iter().skip(3).map(|s| uuid(Some(s))).collect(),
        },
        "load" => VoiceReport::Load {
            server: uuid(rest.get(1)),
            participants: rest
                .get(2)
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| usage()),
        },
        "session-started" => VoiceReport::SessionStarted {
            server: uuid(rest.get(1)),
            session: uuid(rest.get(2)),
            channel: uuid(rest.get(3)),
        },
        "joined" => VoiceReport::ParticipantJoined {
            session: uuid(rest.get(1)),
            channel: uuid(rest.get(2)),
            user: uuid(rest.get(3)),
        },
        "left" => VoiceReport::ParticipantLeft {
            session: uuid(rest.get(1)),
            channel: uuid(rest.get(2)),
            user: uuid(rest.get(3)),
        },
        "speaking" => VoiceReport::Speaking {
            session: uuid(rest.get(1)),
            channel: uuid(rest.get(2)),
            user: uuid(rest.get(3)),
            speaking: flag(rest.get(4)),
        },
        "state" => VoiceReport::ParticipantState {
            session: uuid(rest.get(1)),
            channel: uuid(rest.get(2)),
            user: uuid(rest.get(3)),
            muted: flag(rest.get(4)),
            deafened: flag(rest.get(5)),
            sharing_screen: rest.get(6).is_some_and(|v| v == "true"),
        },
        "ended" => VoiceReport::SessionEnded {
            session: uuid(rest.get(1)),
            channel: uuid(rest.get(2)),
        },
        _ => usage(),
    };
    let server = match &report {
        VoiceReport::Load { server, .. }
        | VoiceReport::SessionStarted { server, .. }
        | VoiceReport::SessionSnapshot { server, .. }
        | VoiceReport::SessionsHeld { server, .. } => *server,
        _ => sender.unwrap_or_else(|| usage()),
    };
    let mut options = async_nats::ConnectOptions::new();
    if let Some(token) = nats_token {
        options = options.token(token);
    }
    let client = async_nats::connect_with_options(&nats_url, options)
        .await
        .expect("connect to NATS");
    async_nats::jetstream::new(client)
        .publish(
            report.subject(server),
            serde_json::to_vec(&report).expect("serializes").into(),
        )
        .await
        .expect("publish")
        .await
        .expect("the report stream took the report");
    println!(
        "sent {}",
        serde_json::to_string(&report).expect("serializes")
    );
}
