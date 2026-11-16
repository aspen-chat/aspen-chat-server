//! One peer connection: a str0m `Rtc` in RTP mode, its negotiation with the client, and what each
//! of its sections carries.
//!
//! Every call into str0m goes through `guarded`, which catches a panic (str0m panics only on its
//! own broken invariants, never on input) so that one connection's fault drops that connection
//! alone, not the thread and every call on it.

use crate::sdp::{FormatEdit, edit_format_parameters};
use crate::{Failure, ParticipantId};
use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};
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

/// Whether a section is one the server receives on, which only another such section can take
/// over: the other way round, its direction would change from receiving to sending.
fn sending(content: &SectionContent) -> bool {
    matches!(content, SectionContent::Sending { .. })
}

/// What a section is for, as negotiated or offered.
#[derive(Debug, Clone)]
pub struct Sectioned {
    pub mid: Mid,
    pub kind: MediaKind,
    /// What it carries, or carried last while `idle`.
    pub content: SectionContent,
    /// The MediaStream a receiver plays it in, for a section the server sends on: a receiver
    /// synchronizes the audio and video of one stream.
    pub stream: Option<String>,
    /// Kept inactive for the next section of its kind and role, rather than stopped: a stopped
    /// section stays in every later offer, so a connection that stopped one per change would
    /// offer ever more.
    pub idle: bool,
}

/// A section a connection should hold: its kind, what it carries, and for a section the server
/// sends on, the stream a receiver plays it in (`None` keeps the one it has).
#[derive(Debug, Clone, PartialEq)]
pub struct Want {
    pub kind: MediaKind,
    pub content: SectionContent,
    pub stream: Option<String>,
}

/// How much of what a sending stream sends in a second may be resends, before str0m drops the
/// ones still queued: half, so a receiver that asks again for packets still on their way over a
/// slow link (one tenth lost, say, a round trip of a few hundred milliseconds) has its asking
/// answered, while one that asks for everything makes the server send at most twice the media.
/// str0m's own default, 0.15, drops most of what such a link needs repaired, and the picture
/// freezes until a keyframe.
const RESEND_SHARE_CAP: f32 = 0.5;
/// What a sending stream keeps to resend from: str0m's own defaults.
const RESEND_CACHE_PACKETS: usize = 1024;
const RESEND_CACHE_AGE: Duration = Duration::from_secs(3);

pub struct Connection {
    pub participant: ParticipantId,
    pub kind: ConnectionKind,
    rtc: Rtc,
    /// The sections whose sending stream has its resend cache set (`RESEND_SHARE_CAP`).
    resending: HashSet<Mid>,
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
    /// The sections the latest offer added, gave other content, or moved into another stream:
    /// what each carries starts over, once that offer is answered.
    pub restarted: Vec<Mid>,
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
            resending: HashSet::new(),
            sections: Vec::new(),
            restarted: Vec::new(),
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
    ///
    /// A section no longer wanted goes idle (inactive), and the next one wanted of its kind and
    /// role takes it over, given its content and stream; only when none is idle is one added. So
    /// a connection never holds more sections than it once needed at once.
    pub fn negotiate(&mut self, wanted: &[Want], now: Instant) -> Option<(u64, String)> {
        if self.pending.is_some() || self.failed.is_some() {
            return None;
        }
        let held = |s: &Sectioned, w: &Want| !s.idle && s.content == w.content;
        let going: Vec<Mid> = self
            .sections
            .iter()
            .filter(|s| !s.idle && !wanted.iter().any(|w| held(s, w)))
            .map(|s| s.mid)
            .collect();
        let moved: Vec<(Mid, String)> = wanted
            .iter()
            .filter_map(|w| {
                let section = self.sections.iter().find(|s| held(s, w))?;
                let stream = w.stream.as_ref()?;
                (section.stream.as_ref() != Some(stream)).then(|| (section.mid, stream.clone()))
            })
            .collect();
        let coming: Vec<&Want> = wanted
            .iter()
            .filter(|w| !self.sections.iter().any(|s| held(s, w)))
            .collect();
        if going.is_empty()
            && moved.is_empty()
            && coming.is_empty()
            && !self.restart_ice
            && self.revision > 0
        {
            return None;
        }
        // Sections going idle, and those already idle, are taken over before any is added.
        let mut free: Vec<(Mid, MediaKind, bool)> = self
            .sections
            .iter()
            .filter(|s| s.idle || going.contains(&s.mid))
            .map(|s| (s.mid, s.kind, sending(&s.content)))
            .collect();
        let mut placed: Vec<(Option<Mid>, Want)> = Vec::new();
        for want in coming {
            let role = sending(&want.content);
            let taken = free
                .iter()
                .position(|(_, kind, r)| *kind == want.kind && *r == role)
                .map(|at| free.remove(at).0);
            placed.push((taken, want.clone()));
        }
        let idling: Vec<Mid> = free
            .iter()
            .map(|(mid, _, _)| *mid)
            .filter(|mid| !self.sections.iter().any(|s| s.mid == *mid && s.idle))
            .collect();
        let restart = std::mem::take(&mut self.restart_ice);
        let applied = self.guarded(|rtc| {
            let mut change = rtc.sdp_api();
            let mut added = Vec::new();
            for (taken, want) in &placed {
                let direction = if sending(&want.content) {
                    Direction::RecvOnly
                } else {
                    Direction::SendOnly
                };
                match taken {
                    Some(mid) => {
                        change.set_direction(*mid, direction);
                        if let Some(stream) = &want.stream {
                            change.set_stream_id(*mid, stream);
                        }
                    }
                    None => {
                        let rtc_kind = match want.kind {
                            MediaKind::Audio => RtcKind::Audio,
                            MediaKind::Video => RtcKind::Video,
                        };
                        added.push(change.add_media(
                            rtc_kind,
                            direction,
                            want.stream.clone(),
                            None,
                            None,
                        ));
                    }
                }
            }
            for mid in &idling {
                change.set_direction(*mid, Direction::Inactive);
            }
            for (mid, stream) in &moved {
                change.set_stream_id(*mid, stream);
            }
            // The fragment a restart's checks name; str0m takes it up with the answer.
            let restarted = restart.then(|| change.ice_restart(true).ufrag);
            (added, restarted, change.apply())
        })?;
        let (added, restarted, offer) = applied;
        let mut changed = Vec::new();
        let mut added = added.into_iter();
        for (taken, want) in placed {
            let mid = match taken {
                Some(mid) => {
                    let section = self.sections.iter_mut().find(|s| s.mid == mid)?;
                    section.content = want.content;
                    section.idle = false;
                    if want.stream.is_some() {
                        section.stream = want.stream;
                    }
                    mid
                }
                None => {
                    let mid = added.next()?;
                    self.sections.push(Sectioned {
                        mid,
                        kind: want.kind,
                        content: want.content,
                        stream: want.stream,
                        idle: false,
                    });
                    mid
                }
            };
            changed.push(mid);
        }
        for section in &mut self.sections {
            if idling.contains(&section.mid) {
                section.idle = true;
            }
            if let Some((mid, stream)) = moved.iter().find(|(mid, _)| *mid == section.mid) {
                section.stream = Some(stream.clone());
                changed.push(*mid);
            }
        }
        // A restarted section may send on new streams of str0m's (one moved into another stream
        // does), which take the resend setting on their first write.
        self.resending.retain(|mid| !changed.contains(mid));
        self.restarted = changed;
        let (offer, pending) = offer?;
        self.touch(now);
        self.pending = Some(pending);
        self.offered_at = Some(now);
        self.revision += 1;
        if let Some(ufrag) = restarted {
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
        for section in self.sections.iter().filter(|s| !s.idle) {
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

    /// What every section that carries something carries; an idle one is left out, so a
    /// client takes what it carried as gone.
    pub fn sections_for_client(&self) -> Vec<Section> {
        self.sections
            .iter()
            .filter(|s| !s.idle)
            .map(|s| Section {
                mid: s.mid.to_string(),
                kind: s.kind,
                content: s.content.clone(),
            })
            .collect()
    }

    /// The section `mid` names while it carries something; an idle one carries nothing.
    pub fn section(&self, mid: Mid) -> Option<&Sectioned> {
        self.sections.iter().find(|s| s.mid == mid && !s.idle)
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
        let first = !self.resending.contains(&mid);
        let written = self
            .guarded(|rtc| {
                let mut api = rtc.direct_api();
                match api.stream_tx_by_mid(mid, None) {
                    Some(stream) => {
                        if first {
                            stream.set_rtx_cache(
                                RESEND_CACHE_PACKETS,
                                RESEND_CACHE_AGE,
                                Some(RESEND_SHARE_CAP),
                            );
                        }
                        stream.write_rtp(write);
                        true
                    }
                    None => false,
                }
            })
            .unwrap_or(false);
        if written && first {
            self.resending.insert(mid);
        }
        written
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

#[cfg(test)]
mod tests {
    use super::*;
    use str0m::change::SdpOffer;
    use str0m::{Candidate, Rtc};

    fn connection() -> (Connection, Rtc) {
        str0m::crypto::from_feature_flags().install_process_default();
        let candidate = Candidate::host("127.0.0.1:40000".parse().unwrap(), "udp").unwrap();
        let connection = Connection::new(ParticipantId(1), ConnectionKind::Call, &[candidate]);
        (connection, Rtc::builder().build(Instant::now()))
    }

    /// Offers `wanted`, has `client` answer, and returns the offer.
    fn offer(connection: &mut Connection, client: &mut Rtc, wanted: &[Want]) -> String {
        let (revision, sdp) = connection.negotiate(wanted, Instant::now()).unwrap();
        let answer = client
            .sdp_api()
            .accept_offer(SdpOffer::from_sdp_string(&sdp).unwrap())
            .unwrap();
        connection
            .accept_answer(revision, &answer.to_sdp_string())
            .unwrap();
        sdp
    }

    fn slot(stream: Option<&str>) -> Want {
        Want {
            kind: MediaKind::Audio,
            content: SectionContent::AudioSlot { slot: 0 },
            stream: stream.map(str::to_string),
        }
    }

    fn camera(user: uuid::Uuid) -> Want {
        Want {
            kind: MediaKind::Video,
            content: SectionContent::Video {
                user,
                source: voice_protocol::signal::MediaSource::Camera,
            },
            stream: Some(format!("{user}-camera")),
        }
    }

    /// Video watched and unwatched over and over takes the section it left idle each time, so
    /// the offer never grows; each time in the stream of whoever it carries now.
    #[test]
    fn a_section_left_idle_is_taken_over_in_its_new_stream() {
        let (mut connection, mut client) = connection();
        let users: Vec<uuid::Uuid> = (0..6).map(|_| uuid::Uuid::now_v7()).collect();
        let mut sizes = Vec::new();
        for (round, user) in users.iter().enumerate() {
            let wanted = if round % 2 == 0 {
                vec![slot(None), camera(*user)]
            } else {
                vec![slot(None)]
            };
            let sdp = offer(&mut connection, &mut client, &wanted);
            sizes.push(sdp.matches("\nm=").count());
            if round % 2 == 0 {
                assert!(sdp.contains(&format!("a=msid:{user}-camera ")), "{sdp}");
                let section = connection
                    .sections
                    .iter()
                    .find(|s| matches!(s.content, SectionContent::Video { .. }))
                    .unwrap();
                assert!(connection.restarted.contains(&section.mid));
            } else {
                assert!(sdp.contains("a=inactive"), "{sdp}");
                assert_eq!(
                    connection.sections_for_client().len(),
                    1,
                    "an idle section is no one's"
                );
            }
        }
        assert!(
            sizes.iter().all(|n| *n == 2),
            "m-lines per offer: {sizes:?}"
        );
    }

    /// A section whose content stays but whose stream changes (a slot's new occupant) is moved
    /// into the new stream in the next offer, and restarts.
    #[test]
    fn a_new_occupant_moves_its_slot_into_their_stream() {
        let (mut connection, mut client) = connection();
        let first = offer(&mut connection, &mut client, &[slot(Some("ann-camera"))]);
        let sdp = offer(&mut connection, &mut client, &[slot(Some("bob-camera"))]);
        assert!(sdp.contains("a=msid:bob-camera "), "{sdp}");
        assert_eq!(sdp.matches("\nm=").count(), 1);
        // On a new SSRC, which a browser makes a new receive stream for, synchronised with the
        // stream it is now in: one it has had stays synchronised with the stream it was in.
        assert_ne!(ssrc_of(&first), ssrc_of(&sdp));
        assert_eq!(connection.restarted.len(), 1);
        // The same occupant again, or none named, offers nothing.
        assert!(
            connection
                .negotiate(&[slot(Some("bob-camera"))], Instant::now())
                .is_none()
        );
        assert!(
            connection
                .negotiate(&[slot(None)], Instant::now())
                .is_none()
        );
    }

    /// The first `a=ssrc` an SDP names.
    fn ssrc_of(sdp: &str) -> String {
        sdp.lines()
            .find_map(|line| line.strip_prefix("a=ssrc:"))
            .and_then(|rest| rest.split(' ').next())
            .expect("an SSRC")
            .to_string()
    }

    /// The `a=ice-ufrag` an SDP names.
    fn ufrag_of(sdp: &str) -> String {
        sdp.lines()
            .find_map(|line| line.strip_prefix("a=ice-ufrag:"))
            .expect("an ICE fragment")
            .trim()
            .to_string()
    }

    /// A browser that restarts ICE checks from a port of its new generation, so its checks are
    /// found by fragment alone: the connection must be known by the one its latest offer names.
    #[test]
    fn a_restart_is_known_by_the_fragment_its_offer_names() {
        str0m::crypto::from_feature_flags().install_process_default();
        let candidate = Candidate::host("127.0.0.1:40000".parse().unwrap(), "udp").unwrap();
        let mut connection = Connection::new(ParticipantId(1), ConnectionKind::Call, &[candidate]);
        let mut client = Rtc::builder().build(Instant::now());
        let slot = [Want {
            kind: MediaKind::Audio,
            content: SectionContent::AudioSlot { slot: 0 },
            stream: None,
        }];
        let (revision, first) = connection.negotiate(&slot, Instant::now()).unwrap();
        assert_eq!(connection.ufrag, ufrag_of(&first));
        let answer = client
            .sdp_api()
            .accept_offer(SdpOffer::from_sdp_string(&first).unwrap())
            .unwrap();
        connection
            .accept_answer(revision, &answer.to_sdp_string())
            .unwrap();

        connection.restart_ice = true;
        let (_, restarted) = connection.negotiate(&slot, Instant::now()).unwrap();
        assert_ne!(ufrag_of(&restarted), ufrag_of(&first));
        assert_eq!(connection.ufrag, ufrag_of(&restarted));
    }
}
