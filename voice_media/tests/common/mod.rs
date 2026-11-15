//! Participants for the media threads' tests: each a synthetic peer (`voice_media::peer`) whose
//! offers and answers the test passes to and from the media thread in process.

#![allow(dead_code)]

use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;
use voice_media::peer::{Peer as Client, PeerOptions, Received, Synthetic};
use voice_media::{CallId, Command, Event, Media, MediaAddress, MediaConfig, ParticipantId};
use voice_protocol::signal::{
    ConnectionKind, MediaSource, Section, SectionContent, SlotAssignment,
};

pub fn media(audio_slots: u8) -> Media {
    Media::start(MediaConfig {
        addresses: vec![MediaAddress {
            ip: "127.0.0.1".parse().unwrap(),
            announced: None,
        }],
        first_port: 0,
        threads: 1,
        audio_slots,
        max_watched_video: 25,
    })
    .expect("media threads start")
}

/// A participant in a test: its id on the media thread, its events, and its client.
pub struct Peer {
    pub id: ParticipantId,
    pub user: Uuid,
    pub events: mpsc::UnboundedReceiver<Event>,
    client: Client,
    /// The game capture helper's client, made with the first capture offer.
    capture: Option<Client>,
    /// The sections of the latest offer of each connection.
    pub sections: Vec<Section>,
    pub capture_sections: Vec<Section>,
    pub slots: HashMap<u8, Option<Uuid>>,
    pub log: Vec<Event>,
}

impl Peer {
    pub fn join(media: &Media, call: CallId, id: u64) -> Self {
        Self::join_over(media, call, id, false)
    }

    /// Joins with a client that reaches the server over TCP alone, as one whose network blocks
    /// UDP does.
    pub fn join_over_tcp(media: &Media, call: CallId, id: u64) -> Self {
        Self::join_over(media, call, id, true)
    }

    fn join_over(media: &Media, call: CallId, id: u64, tcp: bool) -> Self {
        let (events_tx, events) = mpsc::unbounded_channel();
        let user = Uuid::now_v7();
        let client = Client::start(PeerOptions { tcp, detail: true });
        assert!(media.send(
            0,
            Command::Join {
                call,
                participant: ParticipantId(id),
                user,
                events: events_tx,
            },
        ));
        Self {
            id: ParticipantId(id),
            user,
            events,
            client,
            capture: None,
            sections: Vec::new(),
            capture_sections: Vec::new(),
            slots: HashMap::new(),
            log: Vec::new(),
        }
    }

    /// Reads events, answering every offer as a browser would, until `want` matches one or
    /// `within` passes.
    pub async fn until(
        &mut self,
        media: &Media,
        within: Duration,
        want: impl Fn(&Event) -> bool,
    ) -> Option<Event> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let event = tokio::time::timeout_at(deadline, self.events.recv())
                .await
                .ok()??;
            self.log.push(event.clone());
            match &event {
                Event::Offer {
                    connection,
                    revision,
                    sdp,
                    sections,
                    ..
                } => {
                    let client = match connection {
                        ConnectionKind::Call => {
                            self.sections = sections.clone();
                            &self.client
                        }
                        ConnectionKind::Capture => {
                            self.capture_sections = sections.clone();
                            self.capture
                                .get_or_insert_with(|| Client::start(PeerOptions::default()))
                        }
                    };
                    let answer = client
                        .answer(sdp.clone())
                        .await
                        .expect("the client answers");
                    assert!(media.send(
                        0,
                        Command::Answer {
                            participant: self.id,
                            connection: *connection,
                            revision: *revision,
                            sdp: answer,
                        },
                    ));
                }
                Event::Slots { assignments, .. } => {
                    for SlotAssignment { slot, user } in assignments {
                        self.slots.insert(*slot, *user);
                    }
                }
                _ => {}
            }
            if want(&event) {
                return Some(event);
            }
        }
    }

    /// Settles whatever offers are outstanding.
    pub async fn settle(&mut self, media: &Media) {
        self.until(media, Duration::from_millis(400), |_| false)
            .await;
    }

    pub fn mid_of(&self, content: &SectionContent) -> Option<String> {
        self.sections
            .iter()
            .find(|s| &s.content == content)
            .map(|s| s.mid.clone())
    }

    pub fn capture_mid_of(&self, content: &SectionContent) -> Option<String> {
        self.capture_sections
            .iter()
            .find(|s| &s.content == content)
            .map(|s| s.mid.clone())
    }

    pub fn send_audio(&self, mid: &str, level: Option<i8>) {
        self.client
            .send(mid, Synthetic::Audio { kbps: 64.0, level });
    }

    pub fn send_video(&self, mid: &str) {
        self.client.send(mid, Synthetic::SCREEN);
    }

    /// Sends H.264 on a section of the capture connection, as the game capture helper does.
    pub fn capture_video(&self, mid: &str) {
        self.capture
            .as_ref()
            .expect("a capture connection")
            .send(mid, Synthetic::SCREEN);
    }

    pub fn stop(&self, mid: &str) {
        self.client.stop(mid);
    }

    pub async fn ssrc(&self, mid: &str) -> Option<u32> {
        self.client.ssrc(mid).await
    }

    /// Every packet received so far.
    pub fn drain(&mut self) -> Vec<Received> {
        self.client.drain()
    }
}

pub fn send(media: &Media, command: Command) {
    assert!(media.send(0, command));
}

/// The sending section of `source` in an offer's sections.
pub fn sending(source: MediaSource) -> SectionContent {
    SectionContent::Sending { source }
}
