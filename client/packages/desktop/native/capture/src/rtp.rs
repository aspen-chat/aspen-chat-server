//! The capture connection: a WebRTC peer (str0m) of the voice server's own, holding the
//! connection the server offers for what this helper sends, and the senders that hand it encoded
//! frames (H.264 NAL units or Opus packets, one sender per stream).
//!
//! The renderer passes the server's offers through the shell (`connect` for the first,
//! `renegotiate` for each after, an ICE restart among them) and the answers back; the helper
//! then reaches the server itself, by ICE and DTLS over UDP from one socket on the interface its
//! route to the server takes. str0m packetizes the frames, answers NACKs from what it sent,
//! writes the sender reports, and estimates the bandwidth from the server's transport-wide
//! feedback; that estimate becomes the video encoder's bitrate, within the limits the capture
//! was started with. Keyframe requests are counted and otherwise left to the short keyframe
//! interval, since libobs has no way to force a keyframe on demand.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use str0m::bwe::{Bitrate, BweKind};
use str0m::change::SdpOffer;
use str0m::format::Codec;
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};

/// What a sender carries, which decides its codec, its section, and its clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    /// H.264 in Annex B NAL units, a 90 kHz clock.
    Video,
    /// Opus frames, a 48 kHz clock.
    Audio,
}

impl StreamKind {
    fn clock_rate(self) -> Frequency {
        match self {
            StreamKind::Video => Frequency::NINETY_KHZ,
            StreamKind::Audio => Frequency::FORTY_EIGHT_KHZ,
        }
    }

    fn media_kind(self) -> MediaKind {
        match self {
            StreamKind::Video => MediaKind::Video,
            StreamKind::Audio => MediaKind::Audio,
        }
    }

    fn codec(self) -> Codec {
        match self {
            StreamKind::Video => Codec::H264,
            StreamKind::Audio => Codec::Opus,
        }
    }
}

/// A change the bandwidth estimate asks for, applied by the capture on its own thread.
pub trait Feedback: Send + Sync {
    fn set_bitrate_kbps(&self, kbps: u32);
}

/// What the estimate starts at, before the server's feedback says more: a share's picture is
/// sharp from its first seconds, and the estimate comes down within a second where it must.
const INITIAL_BITRATE_KBPS: u64 = 6_000;
/// How often at most the encoder bitrate follows the estimate.
const BITRATE_UPDATE_INTERVAL: Duration = Duration::from_millis(1500);
/// The longest the network thread waits for a packet before str0m's timers are run again.
const MAX_WAIT: Duration = Duration::from_millis(50);
/// Large enough for any datagram str0m or the server sends.
const DATAGRAM: usize = 2048;

/// Whom the estimate steers: the video encoder, within its limits.
struct Steering {
    feedback: Arc<dyn Feedback>,
    min_kbps: u32,
    max_kbps: u32,
    last: Option<Instant>,
}

struct State {
    rtc: Rtc,
    /// The mids of the latest offer's sections, which the senders choose theirs among.
    mids: Vec<Mid>,
    steering: Option<Steering>,
}

/// One capture's connection to the voice server.
pub struct Connection {
    state: Mutex<State>,
    socket: UdpSocket,
    local: SocketAddr,
    stopped: AtomicBool,
    keyframes_wanted: AtomicU64,
}

/// The capture's connection while one runs; one capture runs at a time.
static CURRENT: Mutex<Option<Arc<Connection>>> = Mutex::new(None);

/// The voice server's UDP candidates in an offer, highest priority first as it lists them.
fn udp_candidates(sdp: &str) -> Vec<SocketAddr> {
    sdp.lines()
        .filter_map(|line| line.trim().strip_prefix("a=candidate:"))
        .filter_map(|candidate| Candidate::from_sdp_string(&format!("candidate:{candidate}")).ok())
        .filter(|candidate| candidate.proto() == Protocol::Udp)
        .map(|candidate| candidate.addr())
        .collect()
}

/// The mids of an offer's media sections, in its order.
fn mids(sdp: &str) -> Vec<Mid> {
    sdp.lines()
        .filter_map(|line| line.trim().strip_prefix("a=mid:"))
        .map(Mid::from)
        .collect()
}

/// The address of this machine's interface that the route to `remote` leaves by: what a
/// connected UDP socket is bound to, which sends nothing.
fn route_to(remote: SocketAddr) -> std::io::Result<IpAddr> {
    let unspecified: IpAddr = if remote.is_ipv4() {
        Ipv4Addr::UNSPECIFIED.into()
    } else {
        Ipv6Addr::UNSPECIFIED.into()
    };
    let probe = UdpSocket::bind((unspecified, 0))?;
    probe.connect(remote)?;
    Ok(probe.local_addr()?.ip())
}

/// Answers the first offer of a capture's connection, ending any connection before it, and
/// starts reaching the server.
pub fn connect(offer: &str) -> Result<String, String> {
    close();
    let parsed = SdpOffer::from_sdp_string(offer).map_err(|e| format!("the offer: {e}"))?;
    let (ip, remote) = udp_candidates(offer)
        .into_iter()
        .find_map(|remote| route_to(remote).ok().map(|ip| (ip, remote)))
        .ok_or("the voice server offered no UDP address this machine can reach")?;
    let socket = UdpSocket::bind((ip, 0)).map_err(|e| format!("UDP socket on {ip}: {e}"))?;
    let local = socket
        .local_addr()
        .map_err(|e| format!("UDP socket: {e}"))?;
    socket
        .set_read_timeout(Some(MAX_WAIT))
        .map_err(|e| format!("UDP socket: {e}"))?;
    // str0m's DTLS and SRTP in Rust alone; installing it again does nothing.
    str0m::crypto::from_feature_flags().install_process_default();
    let mut rtc = Rtc::builder()
        .clear_codecs()
        .enable_h264(true)
        .enable_opus(true, false)
        .enable_bwe(Some(Bitrate::kbps(INITIAL_BITRATE_KBPS)))
        .set_stats_interval(None)
        .build(Instant::now());
    let candidate = Candidate::host(local, "udp").map_err(|e| format!("candidate: {e}"))?;
    rtc.add_local_candidate(candidate);
    let answer = rtc
        .sdp_api()
        .accept_offer(parsed)
        .map_err(|e| format!("the offer cannot be answered: {e}"))?
        .to_sdp_string();
    let connection = Arc::new(Connection {
        state: Mutex::new(State {
            rtc,
            mids: mids(offer),
            steering: None,
        }),
        socket,
        local,
        stopped: AtomicBool::new(false),
        keyframes_wanted: AtomicU64::new(0),
    });
    connection.touch();
    let network = Arc::clone(&connection);
    std::thread::Builder::new()
        .name("aspen-rtc".into())
        .spawn(move || network.run())
        .map_err(|e| format!("network thread: {e}"))?;
    eprintln!("aspen-capture: reaching the voice server at {remote} from {local}");
    *CURRENT.lock().expect("connection lock") = Some(connection);
    Ok(answer)
}

/// Answers a later offer of the running capture's connection: a section added or taken away,
/// or an ICE restart.
pub fn renegotiate(offer: &str) -> Result<String, String> {
    let connection = CURRENT
        .lock()
        .expect("connection lock")
        .clone()
        .ok_or("no capture is running")?;
    let parsed = SdpOffer::from_sdp_string(offer).map_err(|e| format!("the offer: {e}"))?;
    let answer = {
        let mut state = connection.state.lock().expect("state lock");
        let answer = state
            .rtc
            .sdp_api()
            .accept_offer(parsed)
            .map_err(|e| format!("the offer cannot be answered: {e}"))?
            .to_sdp_string();
        state.mids = mids(offer);
        answer
    };
    connection.touch();
    Ok(answer)
}

/// Ends the capture's connection, if one runs.
pub fn close() {
    let taken = CURRENT.lock().expect("connection lock").take();
    if let Some(connection) = taken {
        connection.stopped.store(true, Ordering::SeqCst);
        connection
            .state
            .lock()
            .expect("state lock")
            .rtc
            .disconnect();
        let wanted = connection.keyframes_wanted.load(Ordering::Relaxed);
        if wanted > 0 {
            eprintln!("aspen-capture: the voice server asked for {wanted} keyframes");
        }
    }
}

impl Connection {
    /// Lets str0m act on whatever was just asked of it, and sends what that gives: str0m
    /// schedules from its last input, so a change made through its API waits until the next.
    fn touch(&self) {
        let mut state = self.state.lock().expect("state lock");
        if state
            .rtc
            .handle_input(Input::Timeout(Instant::now()))
            .is_ok()
        {
            self.drain(&mut state);
        }
    }

    /// Polls str0m until it waits, sending each transmission and handling each event; returns
    /// when it next needs time.
    fn drain(&self, state: &mut State) -> Option<Instant> {
        loop {
            match state.rtc.poll_output() {
                Ok(Output::Timeout(at)) => return Some(at),
                Ok(Output::Transmit(transmit)) => {
                    let _ = self
                        .socket
                        .send_to(&transmit.contents, transmit.destination);
                }
                Ok(Output::Event(event)) => self.event(state, event),
                Err(error) => {
                    eprintln!("aspen-capture: the connection failed: {error}");
                    self.stopped.store(true, Ordering::SeqCst);
                    return None;
                }
            }
        }
    }

    fn event(&self, state: &mut State, event: Event) {
        match event {
            Event::Connected => eprintln!("aspen-capture: connected to the voice server"),
            Event::KeyframeRequest(_) => {
                self.keyframes_wanted.fetch_add(1, Ordering::Relaxed);
            }
            Event::EgressBitrateEstimate(BweKind::Twcc { estimate, .. }) => {
                let Some(steering) = state.steering.as_mut() else {
                    return;
                };
                if steering
                    .last
                    .is_some_and(|last| last.elapsed() < BITRATE_UPDATE_INTERVAL)
                {
                    return;
                }
                let kbps = u32::try_from(estimate.as_u64() / 1000).unwrap_or(u32::MAX);
                steering
                    .feedback
                    .set_bitrate_kbps(kbps.clamp(steering.min_kbps, steering.max_kbps));
                steering.last = Some(Instant::now());
            }
            _ => {}
        }
    }

    /// The network thread: hands str0m each datagram, and its timers their time, until the
    /// capture ends or the connection dies.
    fn run(self: Arc<Self>) {
        let mut buffer = vec![0u8; DATAGRAM];
        while !self.stopped.load(Ordering::SeqCst) {
            match self.socket.recv_from(&mut buffer) {
                Ok((length, source)) => {
                    let mut state = self.state.lock().expect("state lock");
                    let Ok(contents) = buffer[..length].try_into() else {
                        continue;
                    };
                    let input = Input::Receive(
                        Instant::now(),
                        Receive {
                            proto: Protocol::Udp,
                            source,
                            destination: self.local,
                            contents,
                        },
                    );
                    // str0m is polled after each input: what an input gives is lost if another
                    // comes before the poll.
                    if state.rtc.handle_input(input).is_ok() {
                        self.drain(&mut state);
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => {
                    eprintln!("aspen-capture: the capture's socket failed: {error}");
                    break;
                }
            }
            let mut state = self.state.lock().expect("state lock");
            if !state.rtc.is_alive() {
                eprintln!("aspen-capture: the connection to the voice server ended");
                break;
            }
            if state
                .rtc
                .handle_input(Input::Timeout(Instant::now()))
                .is_ok()
            {
                self.drain(&mut state);
            }
        }
    }
}

/// One stream of the capture, sent on the section of the capture connection that carries its
/// kind. The section is looked up with every frame, so a stream whose section the server offers
/// later (a game's sound after its picture) starts as soon as it is answered, and a frame with
/// no section yet, or before the connection is up, is dropped.
pub struct RtpSender {
    connection: Arc<Connection>,
    kind: StreamKind,
    /// The presentation time of the first frame, which the RTP clock counts from.
    first_pts_us: Mutex<Option<f64>>,
}

impl RtpSender {
    /// A sender of `kind` on the running capture's connection; a video sender's encoder
    /// bitrate follows the estimate through `feedback`, from `min_kbps` to `max_kbps`.
    pub fn start(
        kind: StreamKind,
        feedback: Arc<dyn Feedback>,
        min_kbps: u32,
        max_kbps: u32,
    ) -> Result<Arc<Self>, String> {
        let connection = CURRENT
            .lock()
            .expect("connection lock")
            .clone()
            .ok_or("the capture's connection has not been answered")?;
        if kind == StreamKind::Video {
            let mut state = connection.state.lock().expect("state lock");
            state.steering = Some(Steering {
                feedback,
                min_kbps,
                max_kbps,
                last: None,
            });
            state.rtc.bwe().set_bitrate(
                Bitrate::kbps(u64::from(min_kbps)),
                Bitrate::kbps(u64::from(max_kbps)),
            );
        }
        connection.touch();
        Ok(Arc::new(Self {
            connection,
            kind,
            first_pts_us: Mutex::new(None),
        }))
    }

    /// Sends one encoded frame: Annex B NAL units, with the parameter sets in front of
    /// keyframes, or an Opus packet, at the presentation time it was stamped with.
    pub fn send_frame(&self, data: &[u8], pts_us: f64) {
        let since_first = {
            let mut first = self.first_pts_us.lock().expect("pts lock");
            pts_us - *first.get_or_insert(pts_us)
        };
        let rate = self.kind.clock_rate();
        let samples = (since_first.max(0.0) * f64::from(rate.get()) / 1e6) as u64;
        let connection = &self.connection;
        if connection.stopped.load(Ordering::SeqCst) {
            return;
        }
        let mut state = connection.state.lock().expect("state lock");
        let State { rtc, mids, .. } = &mut *state;
        let Some(mid) = mids.iter().copied().find(|&mid| {
            rtc.media(mid).is_some_and(|media| {
                media.kind() == self.kind.media_kind() && media.direction() == Direction::SendOnly
            })
        }) else {
            return;
        };
        let Some(writer) = rtc.writer(mid) else {
            return;
        };
        let Some(pt) = writer
            .payload_params()
            .find(|params| params.spec().codec == self.kind.codec())
            .map(|params| params.pt())
        else {
            return;
        };
        let time = MediaTime::new(samples, rate);
        if writer.write(pt, Instant::now(), time, data).is_ok() {
            connection.drain(&mut state);
        }
    }

    /// Stops steering the encoder; the connection itself ends with the capture (`close`).
    pub fn stop(&self) {
        if self.kind == StreamKind::Video {
            self.connection.state.lock().expect("state lock").steering = None;
        }
    }
}
