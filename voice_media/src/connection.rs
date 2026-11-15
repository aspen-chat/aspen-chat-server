//! One peer connection: a str0m `Rtc` in RTP mode, its negotiation with the client, and what each
//! of its sections carries.
//!
//! Every call into str0m goes through `guarded`, which catches a panic (str0m panics only on its
//! own broken invariants, never on input) so that one connection's fault drops that connection
//! alone, not the thread and every call on it.

use crate::sdp::{FormatEdit, edit_format_parameters};
use crate::{Failure, ParticipantId};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;
use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::format::{Codec, FormatParams};
use str0m::media::{Direction, Frequency, KeyframeRequestKind, MediaKind as RtcKind, Mid};
use str0m::net::Transmit;
use str0m::rtp::{Extension, RtpWrite, Ssrc};
use str0m::{Candidate, Event as RtcEvent, Input, Output, Rtc};
use voice_protocol::signal::{ConnectionKind, MediaKind, MediaSource, Section, SectionContent};

/// The start bitrate, in kbit/s, Chromium's video encoder is told to begin at: a share is sharp
/// from its first seconds rather than climbing from the default.
const VIDEO_START_KBPS: u32 = 10_000;
/// The bitrate a share's sound is sent at, in bit/s.
const SHARE_AUDIO_BPS: u32 = 128_000;

/// Payload types the server offers. Clients answer with these; each connection's own are read
/// back after its answer (`codecs`) all the same.
const OPUS: u8 = 111;
const VP8: u8 = 96;
const VP8_RTX: u8 = 97;
const H264: u8 = 102;
const H264_RTX: u8 = 103;

/// What a section is for, as negotiated or offered.
#[derive(Debug, Clone)]
pub struct Sectioned {
    pub mid: Mid,
    pub kind: MediaKind,
    pub content: SectionContent,
}

pub struct Connection {
    pub participant: ParticipantId,
    pub kind: ConnectionKind,
    rtc: Rtc,
    /// When str0m next wants a timeout; the thread's timer heap holds the latest.
    pub timeout: Instant,
    /// The ICE username fragment clients address this connection by.
    pub ufrag: String,
    pub revision: u64,
    /// The revision of the last answer applied: a section offered later carries nothing yet,
    /// since the client cannot take it before it has answered.
    pub answered: u64,
    pending: Option<SdpPendingOffer>,
    /// The outstanding offer's text, to say again to a client that resumed.
    pub offered_sdp: Option<String>,
    pub offered_at: Option<Instant>,
    /// When the connection was made, for `CONNECT_TIMEOUT`.
    pub created_at: Instant,
    pub restart_ice: bool,
    /// Every section offered (the outstanding offer's included), in order.
    pub sections: Vec<Sectioned>,
    /// Payload type to codec, as negotiated.
    codecs: Vec<(u8, Codec)>,
    /// The section each received SSRC arrives on.
    ssrc_mid: HashMap<u32, Mid>,
    pub connected: bool,
    pub failed: Option<Failure>,
}

/// What a poll gave: str0m's events, every one of which the thread acts on.
pub type Polled = Vec<RtcEvent>;

impl Connection {
    /// A connection with `candidates` as its local candidates, ICE lite as an SFU's are. A
    /// capture connection takes only H.264 for video, which is what the helper encodes.
    pub fn new(participant: ParticipantId, kind: ConnectionKind, candidates: &[Candidate]) -> Self {
        let mut config = Rtc::builder()
            .clear_codecs()
            .clear_extension_map()
            .set_rtp_mode(true)
            .set_ice_lite(true)
            .set_stats_interval(None)
            // str0m's own numbering: browsers answer with whatever the offer gives, and a str0m
            // client (the game capture helper, the bench) keeps its own.
            .set_extension(1, Extension::AudioLevel)
            .set_extension(2, Extension::AbsoluteSendTime)
            .set_extension(3, Extension::TransportSequenceNumber)
            .set_extension(4, Extension::RtpMid);
        let codecs = config.codec_config();
        codecs.add_config(
            OPUS.into(),
            None,
            Codec::Opus,
            Frequency::FORTY_EIGHT_KHZ,
            Some(2),
            FormatParams {
                min_p_time: Some(10),
                use_inband_fec: Some(true),
                use_dtx: Some(true),
                ..Default::default()
            },
        );
        if kind == ConnectionKind::Call {
            codecs.add_config(
                VP8.into(),
                Some(VP8_RTX.into()),
                Codec::Vp8,
                Frequency::NINETY_KHZ,
                None,
                FormatParams::default(),
            );
        }
        codecs.add_h264(H264.into(), Some(H264_RTX.into()), true, 0x42e01f);
        let mut rtc = config.build(Instant::now());
        for candidate in candidates {
            rtc.add_local_candidate(candidate.clone());
        }
        let ufrag = rtc.direct_api().local_ice_credentials().ufrag;
        Self {
            participant,
            kind,
            rtc,
            timeout: Instant::now(),
            ufrag,
            revision: 0,
            answered: 0,
            pending: None,
            offered_sdp: None,
            offered_at: None,
            created_at: Instant::now(),
            restart_ice: false,
            sections: Vec::new(),
            codecs: Vec::new(),
            ssrc_mid: HashMap::new(),
            connected: false,
            failed: None,
        }
    }

    /// Runs `f` on the `Rtc`, catching a panic: the connection is then failed, and nothing is
    /// called on it again.
    fn guarded<T>(&mut self, f: impl FnOnce(&mut Rtc) -> T) -> Option<T> {
        if self.failed.is_some() {
            return None;
        }
        match catch_unwind(AssertUnwindSafe(|| f(&mut self.rtc))) {
            Ok(value) => Some(value),
            Err(_) => {
                tracing::error!(
                    participant = self.participant.0,
                    "str0m panicked; dropping the connection"
                );
                self.failed = Some(Failure::Internal);
                None
            }
        }
    }

    pub fn accepts(&mut self, input: &Input) -> bool {
        self.guarded(|rtc| rtc.accepts(input)).unwrap_or(false)
    }

    pub fn handle_input(&mut self, input: Input) {
        if self
            .guarded(|rtc| rtc.handle_input(input))
            .is_some_and(|r| r.is_err())
        {
            self.failed = Some(Failure::Disconnected);
        }
    }

    /// Polls until str0m waits, handing each transmission to `send`, and returns its events.
    pub fn poll(&mut self, send: &mut dyn FnMut(Transmit)) -> Polled {
        let polled = self.guarded(|rtc| {
            let mut events = Vec::new();
            loop {
                match rtc.poll_output() {
                    Ok(Output::Transmit(transmit)) => send(transmit),
                    Ok(Output::Timeout(at)) => return (events, Some(at), true),
                    Ok(Output::Event(event)) => events.push(event),
                    Err(e) => {
                        tracing::warn!("poll: {e}");
                        return (events, None, false);
                    }
                }
            }
        });
        let Some((events, timeout, ok)) = polled else {
            return Vec::new();
        };
        if let Some(at) = timeout {
            self.timeout = at;
        }
        if !ok || !self.guarded(|rtc| rtc.is_alive()).unwrap_or(false) {
            self.failed.get_or_insert(Failure::Disconnected);
        }
        for event in &events {
            if matches!(event, RtcEvent::Connected) {
                self.connected = true;
            }
        }
        events
    }

    /// Offers the sections `wanted` if they differ from those offered, or an ICE restart is
    /// wanted, and no offer is outstanding. Returns the offer's revision and text.
    pub fn negotiate(
        &mut self,
        wanted: &[(MediaKind, SectionContent)],
        now: Instant,
    ) -> Option<(u64, String)> {
        if self.pending.is_some() || self.failed.is_some() {
            return None;
        }
        let missing: Vec<(MediaKind, SectionContent)> = wanted
            .iter()
            .filter(|(_, content)| !self.sections.iter().any(|s| &s.content == content))
            .cloned()
            .collect();
        let extra: Vec<Mid> = self
            .sections
            .iter()
            .filter(|s| !wanted.iter().any(|(_, content)| content == &s.content))
            .map(|s| s.mid)
            .collect();
        if missing.is_empty() && extra.is_empty() && !self.restart_ice && self.revision > 0 {
            return None;
        }
        let restart = std::mem::take(&mut self.restart_ice);
        let applied = self.guarded(|rtc| {
            let mut change = rtc.sdp_api();
            let added: Vec<(Mid, MediaKind, SectionContent)> = missing
                .into_iter()
                .map(|(kind, content)| {
                    let direction = match content {
                        SectionContent::Sending { .. } => Direction::RecvOnly,
                        _ => Direction::SendOnly,
                    };
                    let rtc_kind = match kind {
                        MediaKind::Audio => RtcKind::Audio,
                        MediaKind::Video => RtcKind::Video,
                    };
                    (
                        change.add_media(rtc_kind, direction, None, None, None),
                        kind,
                        content,
                    )
                })
                .collect();
            for mid in &extra {
                change.stop_media(*mid);
            }
            if restart {
                change.ice_restart(true);
            }
            (added, change.apply())
        })?;
        let (added, offer) = applied;
        self.sections.retain(|s| !extra.contains(&s.mid));
        self.sections
            .extend(
                added
                    .into_iter()
                    .map(|(mid, kind, content)| Sectioned { mid, kind, content }),
            );
        let (offer, pending) = offer?;
        self.touch(now);
        self.pending = Some(pending);
        self.offered_at = Some(now);
        self.revision += 1;
        if restart
            && let Some(ufrag) = self.guarded(|rtc| rtc.direct_api().local_ice_credentials().ufrag)
        {
            self.ufrag = ufrag;
        }
        let edits = self.format_edits();
        let sdp = edit_format_parameters(&offer.to_sdp_string(), &edits);
        self.offered_sdp = Some(sdp.clone());
        Some((self.revision, sdp))
    }

    /// The format parameters each section's codecs are offered with beyond the connection's own.
    fn format_edits(&self) -> Vec<FormatEdit> {
        let mut edits = Vec::new();
        for section in &self.sections {
            let mid = section.mid.to_string();
            match &section.content {
                SectionContent::Sending {
                    source: MediaSource::ScreenAudio,
                } => edits.push(FormatEdit {
                    mid,
                    codec: "opus",
                    params: vec![
                        ("stereo", "1".into()),
                        ("sprop-stereo", "1".into()),
                        ("usedtx", "0".into()),
                        ("maxaveragebitrate", SHARE_AUDIO_BPS.to_string()),
                    ],
                }),
                SectionContent::Sending {
                    source: MediaSource::Screen | MediaSource::Camera,
                } => {
                    for codec in ["VP8", "H264"] {
                        edits.push(FormatEdit {
                            mid: mid.clone(),
                            codec,
                            params: vec![("x-google-start-bitrate", VIDEO_START_KBPS.to_string())],
                        });
                    }
                }
                SectionContent::ShareAudio { .. } => edits.push(FormatEdit {
                    mid,
                    codec: "opus",
                    params: vec![("stereo", "1".into()), ("sprop-stereo", "1".into())],
                }),
                SectionContent::Sending { .. }
                | SectionContent::AudioSlot { .. }
                | SectionContent::Video { .. } => {}
            }
        }
        edits
    }

    /// Applies the client's answer to the outstanding offer of `revision`.
    pub fn accept_answer(&mut self, revision: u64, sdp: &str) -> Result<(), Failure> {
        if revision != self.revision || self.pending.is_none() {
            // An answer to an offer this connection no longer waits on: a client that answered
            // twice, or late. The outstanding offer, if any, still stands.
            return Ok(());
        }
        let answer = SdpAnswer::from_sdp_string(sdp).map_err(|_| Failure::BadAnswer)?;
        let pending = self.pending.take().expect("checked above");
        self.offered_at = None;
        self.offered_sdp = None;
        match self.guarded(|rtc| rtc.sdp_api().accept_answer(pending, answer)) {
            Some(Ok(())) => {
                self.answered = revision;
                self.touch(Instant::now());
            }
            Some(Err(e)) => {
                tracing::warn!("answer: {e}");
                return Err(Failure::BadAnswer);
            }
            None => return Err(Failure::Internal),
        }
        self.codecs = self
            .guarded(|rtc| {
                rtc.codec_config()
                    .params()
                    .iter()
                    .map(|p| (*p.pt(), p.spec().codec))
                    .collect()
            })
            .unwrap_or_default();
        Ok(())
    }

    pub fn sections_for_client(&self) -> Vec<Section> {
        self.sections
            .iter()
            .map(|s| Section {
                mid: s.mid.to_string(),
                kind: s.kind,
                content: s.content.clone(),
            })
            .collect()
    }

    pub fn section(&self, mid: Mid) -> Option<&Sectioned> {
        self.sections.iter().find(|s| s.mid == mid)
    }

    pub fn codec(&self, payload_type: u8) -> Option<Codec> {
        self.codecs
            .iter()
            .find(|(pt, _)| *pt == payload_type)
            .map(|(_, codec)| *codec)
    }

    pub fn payload_type(&self, codec: Codec) -> Option<u8> {
        self.codecs
            .iter()
            .find(|(_, known)| *known == codec)
            .map(|(pt, _)| *pt)
    }

    /// The section a received packet's SSRC arrives on, learned from str0m once and kept.
    pub fn mid_of(&mut self, ssrc: u32) -> Option<Mid> {
        if let Some(mid) = self.ssrc_mid.get(&ssrc) {
            return Some(*mid);
        }
        let mid = self.guarded(|rtc| {
            rtc.direct_api()
                .stream_rx(&Ssrc::from(ssrc))
                .map(|s| s.mid())
        })??;
        self.ssrc_mid.insert(ssrc, mid);
        Some(mid)
    }

    /// Writes one packet to the section `mid` sends on; `false` when it has no stream yet (its
    /// offer not answered).
    pub fn write(&mut self, mid: Mid, write: RtpWrite) -> bool {
        self.guarded(|rtc| {
            let mut api = rtc.direct_api();
            match api.stream_tx_by_mid(mid, None) {
                Some(stream) => {
                    stream.write_rtp(write);
                    true
                }
                None => false,
            }
        })
        .unwrap_or(false)
    }

    pub fn request_keyframe(&mut self, ssrc: u32, kind: KeyframeRequestKind) {
        self.guarded(|rtc| {
            if let Some(stream) = rtc.direct_api().stream_rx(&Ssrc::from(ssrc)) {
                stream.request_keyframe(kind);
            }
        });
        self.touch(Instant::now());
    }

    /// Gives str0m the time after a change made through its API, which it schedules from only
    /// at its next input.
    fn touch(&mut self, now: Instant) {
        self.handle_input(Input::Timeout(now));
    }
}
