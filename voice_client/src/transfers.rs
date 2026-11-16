//! Files sent between the people of a call, over a WebRTC data channel of their own: the
//! engine's side of what the web client's `FileTransfers` does, on the same frames
//! (`voice_protocol::signal`), so a phone and a browser transfer files between them.
//!
//! The voice server keeps the offers and introduces the two sides of each transfer; the bytes
//! never pass through its media path. Once both sides are told `transferStarting`, the sender
//! opens a peer connection with the ICE servers that frame names (the server's STUN, and its
//! TURN relay with credentials for this transfer alone) and a data channel, and the receiver
//! answers; both trickle their candidates through the server. `relayOnly` gathers only relay
//! candidates, so neither side learns the other's address; `directPreferred` gathers direct ones
//! too, and a hole-punched connection wins when one can be made. The data channel is encrypted
//! end to end either way.
//!
//! A file's bytes never cross the platform boundary whole (`files`): the sender reads the picked
//! file a chunk at a time as the channel takes them, pausing while more than `BUFFER_HIGH_BYTES`
//! wait in it, and the receiver writes each chunk into the place its user chose before accepting.
//! The sender's side runs without its user: once they have offered a file, every acceptance
//! starts sending on its own, so an offer works while its sender is away. A transfer outlives
//! its offer; either side may cancel it at any time, which closes it at once. Nothing resumes: a
//! transfer that fails or is cancelled has to be accepted again while the offer stands.
//!
//! Everything here runs on the call's one task (`call`), which hands it every frame about files,
//! every file command, and every event the tasks it spawns send back (`Event`): a transfer's
//! connection and channel reporting, its pump reading and sending, its writer writing, its
//! timers, and the opening of files. So no lock guards this state, and each handler sees the
//! whole of it.

use crate::call::{Internal, Link};
use crate::files::{FileError, FileSink, Files};
use crate::peer_connection::{
    Configuration, ConnectionState, DataChannel, DataChannelObserver, DataChannelState,
    DescriptionKind, IceCandidate, IceServer, PeerConnection, PeerConnectionError,
    PeerConnectionFactory, PeerConnectionKind, PeerConnectionObserver, SessionDescription, StatsOf,
};
use crate::state::{
    EndedBy, FileCommand, FileFailure, FileOfferState, FilesState, TransferRoute, TransferState,
    TransferStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Notify, mpsc};
use tokio::time::Instant;
use uuid::Uuid;
use voice_protocol::signal::{
    ClientMessage, FileOffer, ServerMessage, TransferEnd, TransferLink, TransferMode,
    TransferPolicy, TransferRole,
};

/// The shortest and longest an offer may stand, in seconds, as the voice server holds them
/// (`voice_server::rooms::transfers`) and the web client clamps its own to; an offer asked for
/// outside them is clamped rather than refused.
pub const MIN_OFFER_SECONDS: u32 = 10;
pub const MAX_OFFER_SECONDS: u32 = 60 * 60;
/// The most bytes sent in one data channel message. Well under the 256 KiB a browser's SCTP
/// association takes in one message (`maxMessageSize`), so a browser receives each whole.
pub const CHUNK_BYTES: u64 = 64 * 1024;
/// Sending pauses above this many bytes waiting in the channel, and resumes once the platform
/// says they have fallen to the low mark.
pub const BUFFER_HIGH_BYTES: u64 = 4 * 1024 * 1024;
pub const BUFFER_LOW_BYTES: u64 = 1024 * 1024;
/// How often a transfer's progress moves in the snapshot, at most.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
/// What the receiver sends down the channel once every byte is in, ahead of closing it.
const DONE: &str = "done";
/// How long a sender whose channel closed after its last byte waits for the receiver's word
/// before calling the transfer failed.
pub const VERDICT_GRACE: Duration = Duration::from_secs(5);
/// How often a live transfer's route is looked at again, since ICE may move it.
pub const ROUTE_INTERVAL: Duration = Duration::from_secs(3);
/// The data channel's label, as the web client names its own.
const CHANNEL_LABEL: &str = "file";

/// What a transfer's peer connection carries through the voice server, as the web client writes
/// and reads it: a description as `RTCSessionDescriptionInit`, a candidate as
/// `RTCIceCandidateInit`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Signal {
    Description { description: SessionDescription },
    Candidate { candidate: IceCandidate },
}

/// What the tasks a transfer spawns, and the platform's observers, send back to the call's task.
pub(crate) enum Event {
    /// The picked file an offer names was opened for its size, or could not be.
    FileOpened {
        offer: Uuid,
        result: Result<u64, FileError>,
    },
    /// The place an acceptance names was opened for writing, or could not be.
    SinkOpened {
        offer: Uuid,
        result: Result<Arc<dyn FileSink>, FileError>,
    },
    /// The time an offer stood for ran out on this device's clock; `epoch` is the offer's as it
    /// was told of, so a later offer under the same id is left alone.
    OfferExpired { offer: Uuid, epoch: u64 },
    /// About one transfer, by the number it was given; one about a transfer that has ended is
    /// let go.
    Transfer { transfer: u64, event: TransferEvent },
}

pub(crate) enum TransferEvent {
    Connection(ConnectionState),
    /// The connection gathered a candidate of its own, for the other side.
    Candidate(IceCandidate),
    /// The sender's data channel arrived at the receiver.
    ChannelOpened(Arc<dyn DataChannel>),
    ChannelState(DataChannelState),
    /// A text message on the channel: the receiver's `DONE`.
    Text(String),
    /// A binary message on the channel: a chunk of the file.
    Binary(Vec<u8>),
    /// The pump has sent this many bytes so far.
    Sent(u64),
    /// The pump has sent every byte of the offered size, and the file ended there.
    AllSent,
    /// The pump could not read or send: the file could not be opened or read, is not the size
    /// it was offered as, or the channel refused.
    SendFailed(String),
    /// The writer has written this many bytes so far.
    Received(u64),
    /// More arrived than was offered.
    Overflow,
    /// The writer could not write, or could not keep the finished file.
    WriteFailed(String),
    /// Every byte is written and the file kept.
    Finished,
    /// What the connection's statistics say of its route; `None` while no pair is selected.
    RouteRead(Option<TransferRoute>),
    /// `VERDICT_GRACE` passed since the sender's channel closed with everything sent.
    VerdictDue,
    /// `ROUTE_INTERVAL` passed since the route was last read.
    RouteDue,
}

/// The connection's observer, forwarding to the call's task; the channel the other side opens is
/// read by a `ChannelObserver` of the same transfer.
struct ConnectionObserver {
    transfer: u64,
    internal: mpsc::UnboundedSender<Internal>,
}

impl ConnectionObserver {
    fn send(&self, event: TransferEvent) {
        let _ = self.internal.send(Internal::Files(Event::Transfer {
            transfer: self.transfer,
            event,
        }));
    }
}

impl PeerConnectionObserver for ConnectionObserver {
    fn connection_state_changed(&self, state: ConnectionState) {
        self.send(TransferEvent::Connection(state));
    }

    fn ice_candidate(&self, candidate: IceCandidate) {
        self.send(TransferEvent::Candidate(candidate));
    }

    fn data_channel_opened(&self, channel: Arc<dyn DataChannel>) -> Arc<dyn DataChannelObserver> {
        self.send(TransferEvent::ChannelOpened(channel));
        Arc::new(ChannelObserver {
            transfer: self.transfer,
            internal: self.internal.clone(),
            low: Arc::new(Notify::new()),
        })
    }
}

/// The channel's observer: messages and states go to the call's task, in order; the low-buffer
/// mark wakes the pump directly, since the pump alone waits on it.
struct ChannelObserver {
    transfer: u64,
    internal: mpsc::UnboundedSender<Internal>,
    low: Arc<Notify>,
}

impl ChannelObserver {
    fn send(&self, event: TransferEvent) {
        let _ = self.internal.send(Internal::Files(Event::Transfer {
            transfer: self.transfer,
            event,
        }));
    }
}

impl DataChannelObserver for ChannelObserver {
    fn state_changed(&self, state: DataChannelState) {
        self.send(TransferEvent::ChannelState(state));
    }

    fn binary(&self, data: Vec<u8>) {
        self.send(TransferEvent::Binary(data));
    }

    fn text(&self, text: String) {
        self.send(TransferEvent::Text(text));
    }

    fn buffered_amount_low(&self) {
        self.low.notify_one();
    }
}

/// An offer standing in the call, with the number of its telling, which its expiry is judged by.
struct Offer {
    state: FileOfferState,
    epoch: u64,
}

/// What the receiver's writer is given, in order.
enum Write {
    Chunk(Vec<u8>),
    /// Nothing more comes: keep the file (a file of no bytes, whose channel opening is its end).
    Finish,
}

/// A transfer under way, this device one side of it.
struct Live {
    id: u64,
    state: TransferState,
    /// Bytes moved, which `state.bytes` follows at most every `PROGRESS_INTERVAL`.
    bytes: u64,
    last_report: Instant,
    pc: Arc<dyn PeerConnection>,
    channel: Option<Arc<dyn DataChannel>>,
    /// Whether the other side's description has been applied; candidates before it wait.
    remote_set: bool,
    pending: Vec<IceCandidate>,
    /// The sender's: what it reads, and the pump's wake and stop.
    reference: Option<String>,
    low: Arc<Notify>,
    stopped: Arc<AtomicBool>,
    all_sent: bool,
    /// The receiver's: where it writes, and the writer that does.
    sink: Option<Arc<dyn FileSink>>,
    writer: Option<mpsc::UnboundedSender<Write>>,
    /// Whether the connection has connected, since when the route is read.
    connected: bool,
}

impl Live {
    fn key(&self) -> (Uuid, Uuid) {
        (self.state.offer, self.state.peer)
    }
}

/// The call's files: offers standing, this device's transfers, everyone's links, and the relay's
/// policy, as `FilesState` shows them. Owned by `Call`, which hands it frames, commands, and
/// events and reads `snapshot` after each.
pub(crate) struct Transfers {
    files: Arc<dyn Files>,
    peer_connections: Arc<dyn PeerConnectionFactory>,
    internal: mpsc::UnboundedSender<Internal>,
    next_id: u64,
    me: Option<Uuid>,
    offers: Vec<Offer>,
    live: Vec<Live>,
    finished: Vec<TransferState>,
    /// Places opened for offers this device accepted, until their transfers start.
    sinks: HashMap<Uuid, Arc<dyn FileSink>>,
    /// The picked files this device offered, by offer, while the offer stands or a transfer of
    /// it goes on; and the name each was offered as, for a failure to name it by.
    references: HashMap<Uuid, (String, String)>,
    /// Offers whose file is being opened for its size, and what each asked.
    asked_offers: HashMap<Uuid, Asked>,
    /// Offers sent and not yet answered by `fileOffered` or an `error`, oldest first.
    pending_offers: VecDeque<Uuid>,
    /// Acceptances whose place is being opened: the mode each asked, and the offer's name as it
    /// stood, for a failure to name it by once the offer is gone.
    asked_accepts: HashMap<Uuid, (TransferMode, String)>,
    /// Acceptances sent and not yet answered by `transferStarting` or an `error`, oldest first.
    pending_accepts: VecDeque<Uuid>,
    links: Vec<TransferLink>,
    relay_mbps: Option<u32>,
    failure: Option<FileFailure>,
}

impl Transfers {
    pub(crate) fn new(
        files: Arc<dyn Files>,
        peer_connections: Arc<dyn PeerConnectionFactory>,
        internal: mpsc::UnboundedSender<Internal>,
    ) -> Self {
        Self {
            files,
            peer_connections,
            internal,
            next_id: 0,
            me: None,
            offers: Vec::new(),
            live: Vec::new(),
            finished: Vec::new(),
            sinks: HashMap::new(),
            references: HashMap::new(),
            asked_offers: HashMap::new(),
            pending_offers: VecDeque::new(),
            asked_accepts: HashMap::new(),
            pending_accepts: VecDeque::new(),
            links: Vec::new(),
            relay_mbps: None,
            failure: None,
        }
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub(crate) fn snapshot(&self) -> FilesState {
        FilesState {
            offers: self
                .offers
                .iter()
                .map(|offer| offer.state.clone())
                .collect(),
            transfers: self
                .live
                .iter()
                .map(|live| live.state.clone())
                .chain(self.finished.iter().cloned())
                .collect(),
            links: self.links.clone(),
            relay_mbps: self.relay_mbps,
            failure: self.failure.clone(),
        }
    }

    // --- The call's place -------------------------------------------------------------------

    /// Starts over from a `ready` frame: the call's offers, links, and relay policy. Every
    /// transfer under way is closed (the server ends them for the other side as the place is
    /// taken anew), and the files this device offered are kept for the offers `ready` still
    /// lists as its own, so an offer standing through a resume can still be served.
    pub(crate) fn reset(
        &mut self,
        me: Uuid,
        offers: Vec<FileOffer>,
        links: Vec<TransferLink>,
        policy: TransferPolicy,
    ) {
        let keep: HashSet<Uuid> = offers
            .iter()
            .filter(|offer| offer.from == me)
            .map(|offer| offer.id)
            .collect();
        self.close_all(&keep);
        self.me = Some(me);
        for offer in offers {
            self.offered(offer);
        }
        self.links = links;
        self.relay_mbps = policy.relay_mbps;
    }

    /// Closes every transfer at once, as when leaving the call, and forgets everything but the
    /// files offered under `keep`.
    pub(crate) fn close_all(&mut self, keep: &HashSet<Uuid>) {
        while let Some(live) = self.live.pop() {
            self.stop(live, None);
        }
        for (_, sink) in self.sinks.drain() {
            tokio::spawn(async move { sink.discard().await });
        }
        let gone: Vec<Uuid> = self
            .references
            .keys()
            .filter(|offer| !keep.contains(offer))
            .copied()
            .collect();
        for offer in gone {
            if let Some((reference, _)) = self.references.remove(&offer) {
                self.files.release(reference);
            }
        }
        self.asked_offers.clear();
        self.pending_offers.clear();
        self.asked_accepts.clear();
        self.pending_accepts.clear();
        self.finished.clear();
        self.offers.clear();
        self.links.clear();
        self.failure = None;
    }

    // --- Commands ---------------------------------------------------------------------------

    /// Offers the picked file `reference` names, as `name`, under `offer`: the file is opened
    /// for its size, which the offer states, and the offer sent once it is known; outside a call
    /// there is no one to send it to, which `file_opened` reports.
    pub(crate) fn offer(
        &mut self,
        offer: Uuid,
        reference: String,
        name: String,
        allow_direct: bool,
        valid_for_seconds: u32,
    ) {
        self.failure = None;
        self.references
            .insert(offer, (reference.clone(), name.clone()));
        self.pending_offers.push_back(offer);
        let asked = Asked {
            name,
            allow_direct,
            valid_for_seconds: valid_for_seconds.clamp(MIN_OFFER_SECONDS, MAX_OFFER_SECONDS),
        };
        self.asked_offers.insert(offer, asked);
        let (files, internal) = (self.files.clone(), self.internal.clone());
        tokio::spawn(async move {
            let result = match files.open(reference).await {
                Ok(source) => {
                    let size = source.size();
                    source.close();
                    Ok(size)
                }
                Err(e) => Err(e),
            };
            let _ = internal.send(Internal::Files(Event::FileOpened { offer, result }));
        });
    }

    async fn file_opened(
        &mut self,
        link: Option<&Link>,
        offer: Uuid,
        result: Result<u64, FileError>,
    ) {
        let Some(asked) = self.asked_offers.remove(&offer) else {
            return;
        };
        match (result, link) {
            (Ok(size), Some(link)) => {
                link.send(ClientMessage::OfferFile {
                    offer,
                    name: asked.name,
                    size,
                    allow_direct: asked.allow_direct,
                    valid_for_seconds: asked.valid_for_seconds,
                })
                .await;
            }
            (Ok(_), None) => {
                self.offer_failed(offer, "not in a call".to_string());
            }
            (Err(e), _) => {
                self.offer_failed(offer, e.0);
            }
        }
    }

    /// The offer `offer` never stood: it is reported and its file let go.
    fn offer_failed(&mut self, offer: Uuid, detail: String) {
        self.pending_offers.retain(|pending| *pending != offer);
        if let Some((reference, name)) = self.references.remove(&offer) {
            self.failure = Some(FileFailure {
                command: FileCommand::Offer,
                offer,
                name,
                detail,
            });
            self.files.release(reference);
        }
    }

    /// Takes back one of this device's offers; transfers already started go on.
    pub(crate) async fn withdraw(&mut self, link: Option<&Link>, offer: Uuid) {
        if let Some(link) = link {
            link.send(ClientMessage::WithdrawFile { offer }).await;
        }
    }

    /// Accepts `offer` in `mode`, writing what arrives into the place `destination` names: the
    /// place is opened first, and the acceptance sent once it is.
    pub(crate) fn accept(
        &mut self,
        link: Option<&Link>,
        offer: Uuid,
        mode: TransferMode,
        destination: String,
    ) {
        self.failure = None;
        let name = self.offer_name(offer);
        if link.is_none() {
            self.failure = Some(FileFailure {
                command: FileCommand::Accept,
                offer,
                name,
                detail: "not in a call".to_string(),
            });
            return;
        }
        self.asked_accepts.insert(offer, (mode, name));
        let (files, internal) = (self.files.clone(), self.internal.clone());
        tokio::spawn(async move {
            let result = files.create_sink(destination).await;
            let _ = internal.send(Internal::Files(Event::SinkOpened { offer, result }));
        });
    }

    async fn sink_opened(
        &mut self,
        link: Option<&Link>,
        offer: Uuid,
        result: Result<Arc<dyn FileSink>, FileError>,
    ) {
        let Some((mode, name)) = self.asked_accepts.remove(&offer) else {
            return;
        };
        let sink = match result {
            Ok(sink) => sink,
            Err(e) => {
                self.failure = Some(FileFailure {
                    command: FileCommand::Accept,
                    offer,
                    name,
                    detail: e.0,
                });
                return;
            }
        };
        let Some(link) = link else {
            tokio::spawn(async move { sink.discard().await });
            return;
        };
        // The offer may have gone while the place was being opened: then there is nothing to
        // accept, and the place holds nothing worth keeping.
        if !self
            .offers
            .iter()
            .any(|standing| standing.state.id == offer)
        {
            self.failure = Some(FileFailure {
                command: FileCommand::Accept,
                offer,
                name,
                detail: "the offer no longer stands".to_string(),
            });
            tokio::spawn(async move { sink.discard().await });
            return;
        }
        self.drop_sink(offer);
        self.sinks.insert(offer, sink);
        self.pending_accepts.push_back(offer);
        link.send(ClientMessage::AcceptFile { offer, mode }).await;
    }

    /// The name of the standing offer `offer`, or of the file this device offered under it;
    /// empty for one this device knows nothing of.
    fn offer_name(&self, offer: Uuid) -> String {
        self.offers
            .iter()
            .find(|standing| standing.state.id == offer)
            .map(|standing| standing.state.name.clone())
            .or_else(|| self.references.get(&offer).map(|(_, name)| name.clone()))
            .unwrap_or_default()
    }

    /// Discards the place a sink was opened for, when its transfer will not start.
    fn drop_sink(&mut self, offer: Uuid) {
        if let Some(sink) = self.sinks.remove(&offer) {
            tokio::spawn(async move { sink.discard().await });
        }
    }

    /// Cancels one transfer at once: the other side is told, and it is closed here.
    pub(crate) async fn cancel(&mut self, link: Option<&Link>, offer: Uuid, peer: Uuid) {
        let Some(id) = self.live_id(offer, peer) else {
            return;
        };
        if let Some(link) = link {
            link.send(ClientMessage::EndTransfer {
                offer,
                peer,
                reason: TransferEnd::Cancelled,
            })
            .await;
        }
        self.end(id, TransferStatus::Cancelled, EndedBy::ThisDevice, None);
    }

    /// Forgets a finished transfer.
    pub(crate) fn dismiss(&mut self, offer: Uuid, peer: Uuid) {
        self.finished
            .retain(|finished| !(finished.offer == offer && finished.peer == peer));
    }

    pub(crate) fn clear_failure(&mut self) {
        self.failure = None;
    }

    // --- Frames -----------------------------------------------------------------------------

    /// Takes a frame about files; false for any other.
    pub(crate) async fn frame(&mut self, link: &Link, frame: ServerMessage) -> bool {
        match frame {
            ServerMessage::FileOffered { offer } => {
                self.offered(offer);
            }
            ServerMessage::FileWithdrawn { offer, .. } => {
                self.offers.retain(|standing| standing.state.id != offer);
                self.drop_sink(offer);
                self.release_file(offer);
            }
            ServerMessage::TransferLinkChanged { link, active } => {
                if active {
                    self.links.push(link);
                } else if let Some(at) = self
                    .links
                    .iter()
                    .position(|l| l.sender == link.sender && l.receiver == link.receiver)
                {
                    self.links.remove(at);
                }
            }
            ServerMessage::TransferStarting {
                offer,
                peer,
                role,
                mode,
                name,
                size,
                ice_servers,
            } => {
                self.start(link, offer, peer, role, mode, name, size, ice_servers)
                    .await;
            }
            ServerMessage::TransferSignal {
                offer,
                peer,
                signal,
            } => {
                if let Some(id) = self.live_id(offer, peer) {
                    self.signal(link, id, signal).await;
                }
            }
            ServerMessage::TransferEnded {
                offer,
                peer,
                reason,
            } => {
                if let Some(id) = self.live_id(offer, peer) {
                    self.end(id, reason.into(), EndedBy::Peer, None);
                }
            }
            _ => return false,
        }
        true
    }

    /// An `error` frame answering `offerFile` or `acceptFile` (its `refused`): the oldest such
    /// request not yet answered is the one refused, since the server answers in order and a
    /// request it took is answered by `fileOffered` or `transferStarting` instead.
    pub(crate) fn refused(&mut self, request: &str, detail: &str) -> bool {
        match request {
            "offerFile" => {
                if let Some(offer) = self.pending_offers.pop_front() {
                    self.offer_failed(offer, detail.to_string());
                }
            }
            "acceptFile" => {
                if let Some(offer) = self.pending_accepts.pop_front() {
                    self.failure = Some(FileFailure {
                        command: FileCommand::Accept,
                        offer,
                        name: self.offer_name(offer),
                        detail: detail.to_string(),
                    });
                    self.drop_sink(offer);
                }
            }
            _ => return false,
        }
        true
    }

    fn offered(&mut self, offer: FileOffer) {
        let epoch = self.id();
        let own = Some(offer.from) == self.me;
        if own {
            self.pending_offers.retain(|pending| *pending != offer.id);
        }
        let state = FileOfferState {
            id: offer.id,
            from: offer.from,
            name: offer.name,
            size: offer.size,
            allow_direct: offer.allow_direct,
            expires_at_ms: unix_millis_now().saturating_add(offer.expires_in_ms),
            own,
        };
        match self
            .offers
            .iter_mut()
            .find(|standing| standing.state.id == offer.id)
        {
            Some(standing) => *standing = Offer { state, epoch },
            None => self.offers.push(Offer { state, epoch }),
        }
        let (internal, id, wait) = (
            self.internal.clone(),
            offer.id,
            Duration::from_millis(offer.expires_in_ms),
        );
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let _ = internal.send(Internal::Files(Event::OfferExpired { offer: id, epoch }));
        });
    }

    /// Lets go of an offered file once its offer is gone and nothing is sending it.
    fn release_file(&mut self, offer: Uuid) {
        let standing = self.offers.iter().any(|o| o.state.id == offer);
        let sending = self
            .live
            .iter()
            .any(|live| live.state.offer == offer && live.state.role == TransferRole::Sender);
        if !standing
            && !sending
            && let Some((reference, _)) = self.references.remove(&offer)
        {
            self.files.release(reference);
        }
    }

    // --- Transfers --------------------------------------------------------------------------

    fn live_id(&self, offer: Uuid, peer: Uuid) -> Option<u64> {
        self.live
            .iter()
            .find(|live| live.key() == (offer, peer))
            .map(|live| live.id)
    }

    fn live_mut(&mut self, id: u64) -> Option<&mut Live> {
        self.live.iter_mut().find(|live| live.id == id)
    }

    #[allow(clippy::too_many_arguments)]
    async fn start(
        &mut self,
        link: &Link,
        offer: Uuid,
        peer: Uuid,
        role: TransferRole,
        mode: TransferMode,
        name: String,
        size: u64,
        ice_servers: Vec<voice_protocol::signal::IceServer>,
    ) {
        let sending = role == TransferRole::Sender;
        let reference = self.references.get(&offer).map(|(r, _)| r.clone());
        if sending && reference.is_none() {
            // This device no longer holds the file (it left and came back): nothing to send.
            link.send(ClientMessage::EndTransfer {
                offer,
                peer,
                reason: TransferEnd::Failed,
            })
            .await;
            return;
        }
        let sink = if sending {
            None
        } else {
            self.pending_accepts.retain(|pending| *pending != offer);
            match self.sinks.remove(&offer) {
                Some(sink) => Some(sink),
                None => {
                    // No place was chosen for it (the offer went and came back under its id, or
                    // the server started a transfer this device never accepted): nothing can
                    // be written.
                    link.send(ClientMessage::EndTransfer {
                        offer,
                        peer,
                        reason: TransferEnd::Failed,
                    })
                    .await;
                    return;
                }
            }
        };
        let id = self.id();
        let observer = Arc::new(ConnectionObserver {
            transfer: id,
            internal: self.internal.clone(),
        });
        let configuration = Configuration {
            ice_servers: ice_servers
                .into_iter()
                .map(|server| IceServer {
                    urls: server.urls,
                    username: server.username,
                    credential: server.credential,
                })
                .collect(),
            ice_transport_policy: Some(match mode {
                TransferMode::RelayOnly => "relay".to_string(),
                TransferMode::DirectPreferred => "all".to_string(),
            }),
        };
        let state = TransferState {
            offer,
            peer,
            role,
            mode,
            name,
            size,
            bytes: 0,
            status: TransferStatus::Connecting,
            route: None,
            ended_by: None,
            failure: None,
        };
        let pc = match self
            .peer_connections
            .create(PeerConnectionKind::Transfer, configuration, observer)
            .await
        {
            Ok(pc) => pc,
            Err(e) => {
                if let Some(sink) = sink {
                    tokio::spawn(async move { sink.discard().await });
                }
                link.send(ClientMessage::EndTransfer {
                    offer,
                    peer,
                    reason: TransferEnd::Failed,
                })
                .await;
                self.finished.push(TransferState {
                    status: TransferStatus::Failed,
                    ended_by: Some(EndedBy::ThisDevice),
                    failure: Some(format!("the peer connection could not be made: {e}")),
                    ..state
                });
                self.release_file(offer);
                return;
            }
        };
        self.live.push(Live {
            id,
            state,
            bytes: 0,
            last_report: Instant::now(),
            pc: pc.clone(),
            channel: None,
            remote_set: false,
            pending: Vec::new(),
            reference,
            low: Arc::new(Notify::new()),
            stopped: Arc::new(AtomicBool::new(false)),
            all_sent: false,
            sink,
            writer: None,
            connected: false,
        });
        if sending {
            let low = self
                .live_mut(id)
                .map(|live| live.low.clone())
                .expect("the transfer was just added");
            let observer = Arc::new(ChannelObserver {
                transfer: id,
                internal: self.internal.clone(),
                low,
            });
            let opened: Result<(Arc<dyn DataChannel>, SessionDescription), PeerConnectionError> =
                async {
                    let channel = pc
                        .create_data_channel(CHANNEL_LABEL.to_string(), observer)
                        .await?;
                    channel.set_buffered_amount_low_threshold(BUFFER_LOW_BYTES);
                    let sdp = pc.create_offer(false).await?;
                    let description = SessionDescription {
                        kind: DescriptionKind::Offer,
                        sdp,
                    };
                    pc.set_local_description(description.clone()).await?;
                    Ok((channel, description))
                }
                .await;
            match opened {
                Ok((channel, description)) => {
                    if let Some(live) = self.live_mut(id) {
                        live.channel = Some(channel);
                    }
                    self.send_signal(link, id, Signal::Description { description })
                        .await;
                }
                Err(e) => {
                    self.fail(
                        Some(link),
                        id,
                        format!("the data channel could not be offered: {e}"),
                    )
                    .await;
                }
            }
        }
    }

    async fn send_signal(&self, link: &Link, id: u64, signal: Signal) {
        let Some(live) = self.live.iter().find(|live| live.id == id) else {
            return;
        };
        link.send(ClientMessage::TransferSignal {
            offer: live.state.offer,
            peer: live.state.peer,
            signal: serde_json::to_value(signal).expect("a signal is plain data"),
        })
        .await;
    }

    /// Part of the other side's peer connection: its description (an offer to answer, or the
    /// answer to this side's), or a candidate, held until the description is applied.
    async fn signal(&mut self, link: &Link, id: u64, signal: serde_json::Value) {
        let signal: Signal = match serde_json::from_value(signal) {
            Ok(signal) => signal,
            Err(e) => {
                self.fail(
                    Some(link),
                    id,
                    format!("the other side sent a signal this device does not read: {e}"),
                )
                .await;
                return;
            }
        };
        let Some(live) = self.live_mut(id) else {
            return;
        };
        let pc = live.pc.clone();
        let result: Result<Option<SessionDescription>, _> = match signal {
            Signal::Description { description } => {
                let kind = description.kind;
                live.remote_set = true;
                let pending = std::mem::take(&mut live.pending);
                async {
                    pc.set_remote_description(description).await?;
                    for candidate in pending.into_iter().filter(has_candidate) {
                        pc.add_ice_candidate(candidate).await?;
                    }
                    if kind == DescriptionKind::Offer {
                        let sdp = pc.create_answer().await?;
                        let answer = SessionDescription {
                            kind: DescriptionKind::Answer,
                            sdp,
                        };
                        pc.set_local_description(answer.clone()).await?;
                        return Ok(Some(answer));
                    }
                    Ok(None)
                }
                .await
            }
            Signal::Candidate { candidate } => {
                if !live.remote_set {
                    live.pending.push(candidate);
                    Ok(None)
                } else if has_candidate(&candidate) {
                    pc.add_ice_candidate(candidate).await.map(|()| None)
                } else {
                    Ok(None)
                }
            }
        };
        match result {
            Ok(Some(answer)) => {
                self.send_signal(
                    link,
                    id,
                    Signal::Description {
                        description: answer,
                    },
                )
                .await;
            }
            Ok(None) => {}
            Err(e) => {
                self.fail(
                    Some(link),
                    id,
                    format!("the connection could not be set up: {e}"),
                )
                .await;
            }
        }
    }

    /// An event of one transfer, from its connection, channel, pump, writer, or timers.
    pub(crate) async fn transfer_event(
        &mut self,
        link: Option<&Link>,
        id: u64,
        event: TransferEvent,
    ) {
        let Some(live) = self.live_mut(id) else {
            return;
        };
        match event {
            TransferEvent::Connection(state) => match state {
                ConnectionState::Failed => {
                    self.fail(
                        link,
                        id,
                        "the connection to the other side failed".to_string(),
                    )
                    .await;
                }
                ConnectionState::Connected => {
                    if !live.connected {
                        live.connected = true;
                        self.read_route(id);
                    }
                }
                ConnectionState::New
                | ConnectionState::Connecting
                | ConnectionState::Disconnected
                | ConnectionState::Closed => {}
            },
            TransferEvent::Candidate(candidate) => {
                if let Some(link) = link
                    && has_candidate(&candidate)
                {
                    self.send_signal(link, id, Signal::Candidate { candidate })
                        .await;
                }
            }
            TransferEvent::ChannelOpened(channel) => {
                if live.state.role == TransferRole::Receiver && live.channel.is_none() {
                    live.channel = Some(channel);
                }
            }
            TransferEvent::ChannelState(state) => match state {
                DataChannelState::Open => self.channel_open(id),
                DataChannelState::Closed => {
                    if live.state.role == TransferRole::Sender && live.all_sent {
                        // Everything was sent: the verdict may still be on its way through the
                        // voice server.
                        let internal = self.internal.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(VERDICT_GRACE).await;
                            let _ = internal.send(Internal::Files(Event::Transfer {
                                transfer: id,
                                event: TransferEvent::VerdictDue,
                            }));
                        });
                    } else {
                        self.fail(link, id, "the data channel closed".to_string())
                            .await;
                    }
                }
                DataChannelState::Connecting | DataChannelState::Closing => {}
            },
            TransferEvent::Text(text) => {
                // The receiver's word that every byte arrived, sent down the channel ahead of
                // its closing so that it always comes first; its `endTransfer` tells the server.
                if live.state.role == TransferRole::Sender && text == DONE {
                    self.end(id, TransferStatus::Completed, EndedBy::Peer, None);
                }
            }
            TransferEvent::Binary(chunk) => {
                if let Some(writer) = &live.writer {
                    let _ = writer.send(Write::Chunk(chunk));
                }
            }
            TransferEvent::Sent(bytes) | TransferEvent::Received(bytes) => {
                live.bytes = bytes;
                let now = Instant::now();
                if now.duration_since(live.last_report) >= PROGRESS_INTERVAL {
                    live.last_report = now;
                    live.state.bytes = bytes;
                }
            }
            TransferEvent::AllSent => {
                live.all_sent = true;
                live.state.bytes = live.bytes;
            }
            TransferEvent::SendFailed(detail) | TransferEvent::WriteFailed(detail) => {
                self.fail(link, id, detail).await;
            }
            TransferEvent::Overflow => {
                self.fail(
                    link,
                    id,
                    "more arrived than the file was offered as".to_string(),
                )
                .await;
            }
            TransferEvent::Finished => {
                if let Some(channel) = &live.channel
                    && channel.state() == DataChannelState::Open
                {
                    // Closing a channel sends what it holds first, so this reaches the sender
                    // before the close.
                    let _ = channel.send_text(DONE.to_string());
                }
                let (offer, peer) = live.key();
                if let Some(link) = link {
                    link.send(ClientMessage::EndTransfer {
                        offer,
                        peer,
                        reason: TransferEnd::Completed,
                    })
                    .await;
                }
                self.end(id, TransferStatus::Completed, EndedBy::ThisDevice, None);
            }
            TransferEvent::RouteRead(route) => {
                if route.is_some() && route != live.state.route {
                    live.state.route = route;
                    live.state.bytes = live.bytes;
                }
                let internal = self.internal.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(ROUTE_INTERVAL).await;
                    let _ = internal.send(Internal::Files(Event::Transfer {
                        transfer: id,
                        event: TransferEvent::RouteDue,
                    }));
                });
            }
            TransferEvent::RouteDue => self.read_route(id),
            TransferEvent::VerdictDue => {
                self.fail(
                    link,
                    id,
                    "the other side never said the file arrived".to_string(),
                )
                .await;
            }
        }
    }

    /// The channel is open: the transfer moves. The sender starts its pump; a receiver of a file
    /// of no bytes is done at once.
    fn channel_open(&mut self, id: u64) {
        let internal = self.internal.clone();
        let files = self.files.clone();
        let Some(live) = self.live_mut(id) else {
            return;
        };
        live.state.status = TransferStatus::Moving;
        live.state.bytes = live.bytes;
        match live.state.role {
            TransferRole::Sender => {
                let (Some(channel), Some(reference)) =
                    (live.channel.clone(), live.reference.clone())
                else {
                    return;
                };
                let (low, stopped, size) =
                    (live.low.clone(), live.stopped.clone(), live.state.size);
                tokio::spawn(pump(
                    id, internal, files, reference, size, channel, low, stopped,
                ));
            }
            TransferRole::Receiver => {
                let Some(sink) = live.sink.clone() else {
                    return;
                };
                let (tx, rx) = mpsc::unbounded_channel();
                if live.state.size == 0 {
                    let _ = tx.send(Write::Finish);
                }
                live.writer = Some(tx);
                tokio::spawn(writer(id, internal, sink, live.state.size, rx));
            }
        }
    }

    /// Asks the connection's statistics which route it is on; `RouteRead` brings the answer.
    fn read_route(&self, id: u64) {
        let Some(live) = self.live.iter().find(|live| live.id == id) else {
            return;
        };
        let (pc, internal) = (live.pc.clone(), self.internal.clone());
        tokio::spawn(async move {
            let route = match pc.stats(StatsOf::Connection).await {
                Ok(json) => route_of(&json),
                // A closed connection has no statistics; its last route stands.
                Err(_) => None,
            };
            let _ = internal.send(Internal::Files(Event::Transfer {
                transfer: id,
                event: TransferEvent::RouteRead(route),
            }));
        });
    }

    /// Ends a transfer as failed on this side: the other side is told, and what was written is
    /// discarded.
    async fn fail(&mut self, link: Option<&Link>, id: u64, detail: String) {
        let Some(live) = self.live.iter().find(|live| live.id == id) else {
            return;
        };
        let (offer, peer) = live.key();
        if let Some(link) = link {
            link.send(ClientMessage::EndTransfer {
                offer,
                peer,
                reason: TransferEnd::Failed,
            })
            .await;
        }
        self.end(
            id,
            TransferStatus::Failed,
            EndedBy::ThisDevice,
            Some(detail),
        );
    }

    /// Closes a transfer at once and keeps its final state for the page until dismissed.
    fn end(&mut self, id: u64, status: TransferStatus, ended_by: EndedBy, failure: Option<String>) {
        let Some(at) = self.live.iter().position(|live| live.id == id) else {
            return;
        };
        let live = self.live.remove(at);
        let offer = live.state.offer;
        let finished = self.stop(live, Some((status, ended_by, failure)));
        if let Some(finished) = finished {
            self.finished.push(finished);
        }
        self.release_file(offer);
    }

    /// Releases everything a transfer holds: its pump and writer stopped, its channel and
    /// connection closed, and what it wrote discarded unless it completed. With an `outcome`,
    /// the final state to keep.
    fn stop(
        &self,
        mut live: Live,
        outcome: Option<(TransferStatus, EndedBy, Option<String>)>,
    ) -> Option<TransferState> {
        live.stopped.store(true, Ordering::SeqCst);
        live.low.notify_one();
        live.writer = None;
        if let Some(channel) = &live.channel {
            channel.close();
        }
        live.pc.close();
        let completed = matches!(outcome, Some((TransferStatus::Completed, _, _)));
        if !completed && let Some(sink) = live.sink.clone() {
            tokio::spawn(async move { sink.discard().await });
        }
        let (status, ended_by, failure) = outcome?;
        Some(TransferState {
            bytes: live.bytes,
            status,
            ended_by: Some(ended_by),
            failure,
            ..live.state
        })
    }

    // --- Events -----------------------------------------------------------------------------

    pub(crate) async fn event(&mut self, link: Option<&Link>, event: Event) {
        match event {
            Event::FileOpened { offer, result } => self.file_opened(link, offer, result).await,
            Event::SinkOpened { offer, result } => self.sink_opened(link, offer, result).await,
            Event::OfferExpired { offer, epoch } => {
                if self
                    .offers
                    .iter()
                    .any(|standing| standing.state.id == offer && standing.epoch == epoch)
                {
                    self.offers.retain(|standing| standing.state.id != offer);
                    self.drop_sink(offer);
                    self.release_file(offer);
                }
            }
            Event::Transfer { transfer, event } => self.transfer_event(link, transfer, event).await,
        }
    }
}

/// What `offer` asked, kept while the file is opened for its size.
struct Asked {
    name: String,
    allow_direct: bool,
    valid_for_seconds: u32,
}

/// Whether a candidate is one, rather than a browser's end-of-candidates mark.
fn has_candidate(candidate: &IceCandidate) -> bool {
    !candidate.candidate.is_empty()
}

/// Milliseconds since the Unix epoch on this device's clock, which the page judges offers by.
fn unix_millis_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Reads the offered file and sends it down the channel, `CHUNK_BYTES` at a time, pausing while
/// more than `BUFFER_HIGH_BYTES` wait in the channel until `low` says they have drained, until
/// `stopped`. The file must be the `size` it was offered as: one that ends short, or goes on
/// past it, has changed since it was offered, and the transfer fails rather than deliver a file
/// other than the one offered.
#[allow(clippy::too_many_arguments)]
async fn pump(
    id: u64,
    internal: mpsc::UnboundedSender<Internal>,
    files: Arc<dyn Files>,
    reference: String,
    size: u64,
    channel: Arc<dyn DataChannel>,
    low: Arc<Notify>,
    stopped: Arc<AtomicBool>,
) {
    let report = |event: TransferEvent| {
        let _ = internal.send(Internal::Files(Event::Transfer {
            transfer: id,
            event,
        }));
    };
    let source = match files.open(reference).await {
        Ok(source) => source,
        Err(e) => {
            report(TransferEvent::SendFailed(format!(
                "the file could not be opened: {e}"
            )));
            return;
        }
    };
    let mut position = 0u64;
    loop {
        if stopped.load(Ordering::SeqCst) {
            break;
        }
        if channel.buffered_amount() > BUFFER_HIGH_BYTES {
            low.notified().await;
            continue;
        }
        let want = (size - position).min(CHUNK_BYTES);
        if want == 0 {
            // Every byte of the offered size is on its way. The file must end here too.
            match source.read(1).await {
                Ok(more) if more.is_empty() => report(TransferEvent::AllSent),
                Ok(_) => report(TransferEvent::SendFailed(
                    "the file is longer than it was when offered".to_string(),
                )),
                Err(e) => report(TransferEvent::SendFailed(format!(
                    "the file could not be read: {e}"
                ))),
            }
            break;
        }
        let chunk = match source.read(want).await {
            Ok(chunk) => chunk,
            Err(e) => {
                report(TransferEvent::SendFailed(format!(
                    "the file could not be read: {e}"
                )));
                break;
            }
        };
        if chunk.is_empty() {
            report(TransferEvent::SendFailed(
                "the file is shorter than it was when offered".to_string(),
            ));
            break;
        }
        if stopped.load(Ordering::SeqCst) {
            break;
        }
        let sent = chunk.len() as u64;
        if let Err(e) = channel.send_binary(chunk) {
            report(TransferEvent::SendFailed(format!(
                "the data channel refused a chunk: {e}"
            )));
            break;
        }
        position += sent;
        report(TransferEvent::Sent(position));
    }
    source.close();
}

/// Writes what arrives into the chosen place, in order, until every byte of the offered `size`
/// is in, and then keeps the file. More than `size` fails the transfer; a `Finish` keeps a file
/// of no bytes. Ends when the call's task lets the sender go.
async fn writer(
    id: u64,
    internal: mpsc::UnboundedSender<Internal>,
    sink: Arc<dyn FileSink>,
    size: u64,
    mut writes: mpsc::UnboundedReceiver<Write>,
) {
    let report = |event: TransferEvent| {
        let _ = internal.send(Internal::Files(Event::Transfer {
            transfer: id,
            event,
        }));
    };
    let mut total = 0u64;
    while let Some(write) = writes.recv().await {
        match write {
            Write::Chunk(chunk) => {
                total += chunk.len() as u64;
                if total > size {
                    report(TransferEvent::Overflow);
                    return;
                }
                if let Err(e) = sink.write(chunk).await {
                    report(TransferEvent::WriteFailed(format!(
                        "the file could not be written: {e}"
                    )));
                    return;
                }
                report(TransferEvent::Received(total));
                if total < size {
                    continue;
                }
            }
            Write::Finish => {}
        }
        match sink.finish().await {
            Ok(()) => report(TransferEvent::Finished),
            Err(e) => report(TransferEvent::WriteFailed(format!(
                "the file could not be kept: {e}"
            ))),
        }
        return;
    }
}

/// The route of a connection's selected candidate pair, from its statistics as the W3C array
/// (`PeerConnection::stats`): relayed when either end is a relay candidate. Chromium names the
/// pair from its transport; Firefox marks it nominated; native WebRTC does the former.
pub fn route_of(stats: &str) -> Option<TransferRoute> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Entry {
        id: Option<String>,
        #[serde(rename = "type")]
        kind: Option<String>,
        selected_candidate_pair_id: Option<String>,
        nominated: Option<bool>,
        state: Option<String>,
        local_candidate_id: Option<String>,
        remote_candidate_id: Option<String>,
        candidate_type: Option<String>,
    }
    let entries: Vec<Entry> = serde_json::from_str(stats).ok()?;
    let by_id: HashMap<&str, &Entry> = entries
        .iter()
        .filter_map(|entry| Some((entry.id.as_deref()?, entry)))
        .collect();
    let selected = entries
        .iter()
        .find(|entry| entry.kind.as_deref() == Some("transport"))
        .and_then(|transport| by_id.get(transport.selected_candidate_pair_id.as_deref()?))
        .copied();
    let pair = selected.or_else(|| {
        entries.iter().find(|entry| {
            entry.kind.as_deref() == Some("candidate-pair")
                && entry.nominated == Some(true)
                && entry.state.as_deref() == Some("succeeded")
        })
    })?;
    let local = by_id.get(pair.local_candidate_id.as_deref()?)?;
    let remote = by_id.get(pair.remote_candidate_id.as_deref()?)?;
    let relay = |candidate: &Entry| candidate.candidate_type.as_deref() == Some("relay");
    Some(if relay(local) || relay(remote) {
        TransferRoute::Relayed
    } else {
        TransferRoute::Direct
    })
}

impl From<TransferEnd> for TransferStatus {
    fn from(end: TransferEnd) -> Self {
        match end {
            TransferEnd::Completed => TransferStatus::Completed,
            TransferEnd::Cancelled => TransferStatus::Cancelled,
            TransferEnd::Failed => TransferStatus::Failed,
            TransferEnd::Left => TransferStatus::Left,
            TransferEnd::NotPermitted => TransferStatus::NotPermitted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_route_is_read_in_either_browsers_shape() {
        // Chromium: the transport names the pair.
        let chromium = r#"[
            {"id":"T","type":"transport","selectedCandidatePairId":"P"},
            {"id":"P","type":"candidate-pair","localCandidateId":"L","remoteCandidateId":"R"},
            {"id":"L","type":"local-candidate","candidateType":"srflx"},
            {"id":"R","type":"remote-candidate","candidateType":"relay"}
        ]"#;
        assert_eq!(route_of(chromium), Some(TransferRoute::Relayed));
        // Firefox: the pair is marked nominated.
        let firefox = r#"[
            {"id":"Q","type":"candidate-pair","nominated":false,"state":"succeeded","localCandidateId":"X","remoteCandidateId":"Y"},
            {"id":"P","type":"candidate-pair","nominated":true,"state":"succeeded","localCandidateId":"L","remoteCandidateId":"R"},
            {"id":"L","type":"local-candidate","candidateType":"host"},
            {"id":"R","type":"remote-candidate","candidateType":"srflx"}
        ]"#;
        assert_eq!(route_of(firefox), Some(TransferRoute::Direct));
        assert_eq!(route_of("[]"), None);
        assert_eq!(route_of("not json"), None);
        // A pair whose candidates are missing says nothing.
        let missing = r#"[
            {"id":"T","type":"transport","selectedCandidatePairId":"P"},
            {"id":"P","type":"candidate-pair","localCandidateId":"L","remoteCandidateId":"R"}
        ]"#;
        assert_eq!(route_of(missing), None);
    }

    #[test]
    fn signals_read_and_write_as_the_web_client_does() {
        let description: Signal = serde_json::from_str(
            r#"{"kind":"description","description":{"type":"answer","sdp":"v=0"}}"#,
        )
        .unwrap();
        assert_eq!(
            description,
            Signal::Description {
                description: SessionDescription {
                    kind: DescriptionKind::Answer,
                    sdp: "v=0".to_string(),
                }
            }
        );
        let candidate = Signal::Candidate {
            candidate: IceCandidate {
                candidate: "candidate:1 1 udp 1 1.2.3.4 5 typ host".to_string(),
                sdp_mid: Some("0".to_string()),
                sdp_m_line_index: Some(0),
                username_fragment: None,
            },
        };
        assert_eq!(
            serde_json::to_string(&candidate).unwrap(),
            r#"{"kind":"candidate","candidate":{"candidate":"candidate:1 1 udp 1 1.2.3.4 5 typ host","sdpMid":"0","sdpMLineIndex":0}}"#
        );
        assert!(!has_candidate(&IceCandidate {
            candidate: String::new(),
            sdp_mid: Some("0".to_string()),
            sdp_m_line_index: Some(0),
            username_fragment: None,
        }));
    }
}
