//! Files offered in a call, and the transfers between its participants (the flow is in
//! `voice_protocol::signal`). An offer stands for the time its sender gave it, or until they
//! withdraw it or leave; each acceptance starts a transfer between the sender and the one who
//! accepted, which goes on until either ends it or leaves, whatever becomes of the offer.
//! Everyone in the call is told when a transfer starts and ends, and between whom, but not
//! what it carries. The bytes never pass through here: the two devices open a peer connection
//! of their own, with this server's STUN and TURN (`transfer::Relay`) as its ICE servers. Each
//! offer, and each transfer's start and end, is reported to the API server, which keeps the
//! deployment's record of them: names and sizes, never contents.

use super::{Room, RoomError, Rooms, Seat, seated};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::info;
use uuid::Uuid;
use voice_protocol::control::VoiceReport;
use voice_protocol::signal::{
    FileOffer, OfferEnd, ServerMessage, TransferEnd, TransferLink, TransferMode, TransferRole,
};

/// The shortest and longest time an offer may stand. An hour at most keeps an unattended offer
/// something for the people in a call, not a device left seeding a file to whoever joins.
pub const MIN_VALID_SECONDS: u32 = 10;
pub const MAX_VALID_SECONDS: u32 = 60 * 60;
/// The longest file name offered, in characters.
const MAX_NAME_CHARS: usize = 255;
/// How many offers one participant may have standing at once.
const MAX_OFFERS: usize = 10;
/// How many transfers one participant may be part of at once, sending and receiving together.
const MAX_TRANSFERS: usize = 20;
/// The largest transfer signal passed on, as JSON: a data channel's offer or answer is a few
/// kilobytes, a candidate a few hundred bytes. Each is queued on the other side's socket, so
/// the cap and `transferSignal`'s burst together keep one sender well inside the other's
/// outbox (`OUTBOX_BYTES`).
const MAX_SIGNAL_BYTES: usize = 32 * 1024;

pub struct Offer {
    /// The offer's id in the deployment's record (`VoiceReport`), made here.
    record: Uuid,
    from: Uuid,
    name: String,
    size: u64,
    allow_direct: bool,
    expires: Instant,
}

impl Offer {
    fn wire(&self, id: Uuid) -> FileOffer {
        FileOffer {
            id,
            from: self.from,
            name: self.name.clone(),
            size: self.size,
            allow_direct: self.allow_direct,
            expires_in_ms: u64::try_from(
                self.expires
                    .saturating_duration_since(Instant::now())
                    .as_millis(),
            )
            .unwrap_or(u64::MAX),
        }
    }
}

pub struct Transfer {
    sender: Uuid,
    mode: TransferMode,
    /// Its offer's id in the deployment's record, which the transfer outlives.
    record: Uuid,
}

fn mode_label(mode: TransferMode) -> &'static str {
    match mode {
        TransferMode::DirectPreferred => "directPreferred",
        TransferMode::RelayOnly => "relayOnly",
    }
}

impl Room {
    /// The offers that may still be accepted, for a newcomer.
    pub(super) fn standing_offers(&self) -> Vec<FileOffer> {
        let now = Instant::now();
        self.offers
            .lock()
            .expect("offers lock")
            .iter()
            .filter(|(_, offer)| offer.expires > now)
            .map(|(id, offer)| offer.wire(*id))
            .collect()
    }

    /// Every transfer under way, as everyone in the call sees it.
    pub(super) fn links(&self) -> Vec<TransferLink> {
        self.transfers
            .lock()
            .expect("transfers lock")
            .iter()
            .map(|((_, receiver), transfer)| TransferLink {
                sender: transfer.sender,
                receiver: *receiver,
            })
            .collect()
    }

    /// Sends to one participant; false when they are not in the call.
    fn send_to(&self, user: Uuid, message: ServerMessage) -> bool {
        match self.participants.lock().expect("room lock").get(&user) {
            Some(participant) => {
                participant.send(message);
                true
            }
            None => false,
        }
    }

    /// How many transfers `user` is part of.
    fn transfers_of(&self, user: Uuid) -> usize {
        self.transfers
            .lock()
            .expect("transfers lock")
            .iter()
            .filter(|((_, receiver), transfer)| *receiver == user || transfer.sender == user)
            .count()
    }

    /// The transfer of `offer` between `user` and `peer`, either way round: its key and
    /// which side `user` is.
    fn transfer_between(
        &self,
        offer: Uuid,
        user: Uuid,
        peer: Uuid,
    ) -> Option<((Uuid, Uuid), TransferRole)> {
        let transfers = self.transfers.lock().expect("transfers lock");
        if transfers
            .get(&(offer, user))
            .is_some_and(|t| t.sender == peer)
        {
            return Some(((offer, user), TransferRole::Receiver));
        }
        if transfers
            .get(&(offer, peer))
            .is_some_and(|t| t.sender == user)
        {
            return Some(((offer, peer), TransferRole::Sender));
        }
        None
    }
}

impl Rooms {
    /// Offers a file to the call under `id`, standing for `valid_for_seconds`. The caller has
    /// checked the join token's Transfer files grant.
    #[allow(clippy::too_many_arguments)]
    pub async fn offer_file(
        self: &Arc<Self>,
        seat: Seat,
        id: Uuid,
        name: String,
        size: u64,
        allow_direct: bool,
        valid_for_seconds: u32,
    ) -> Result<(), RoomError> {
        let Seat { channel, user, .. } = seat;
        let room = self.room(channel)?;
        let name = name.trim().to_string();
        if name.is_empty()
            || name.chars().count() > MAX_NAME_CHARS
            || name.chars().any(char::is_control)
        {
            return Err(RoomError::BadParameters(format!(
                "a file's name must be 1 to {MAX_NAME_CHARS} printable characters"
            )));
        }
        if !(MIN_VALID_SECONDS..=MAX_VALID_SECONDS).contains(&valid_for_seconds) {
            return Err(RoomError::BadParameters(format!(
                "an offer stands for {MIN_VALID_SECONDS} to {MAX_VALID_SECONDS} seconds"
            )));
        }
        if !allow_direct && self.relay.policy().relay_mbps.is_none() {
            return Err(RoomError::BadParameters(
                "this server does not relay transfers, so an offer must allow direct ones"
                    .to_string(),
            ));
        }
        let record = Uuid::now_v7();
        let offer = Offer {
            record,
            from: user,
            name,
            size,
            allow_direct,
            expires: Instant::now() + Duration::from_secs(u64::from(valid_for_seconds)),
        };
        let wire = {
            // Under the room's lock, so the offer is either made before its sender leaves, and
            // withdrawn as they do, or refused.
            let participants = room.participants.lock().expect("room lock");
            seated(&participants, seat)?;
            let mut offers = room.offers.lock().expect("offers lock");
            if offers.contains_key(&id) {
                return Err(RoomError::BadParameters(
                    "an offer with that id exists".to_string(),
                ));
            }
            if offers.values().filter(|o| o.from == user).count() >= MAX_OFFERS {
                return Err(RoomError::BadParameters(format!(
                    "at most {MAX_OFFERS} offers may stand at once"
                )));
            }
            let wire = offer.wire(id);
            offers.insert(id, offer);
            wire
        };
        let reported = VoiceReport::FileOffered {
            channel,
            record,
            sender: user,
            name: wire.name.clone(),
            size,
            allow_direct,
            valid_for_seconds,
        };
        room.broadcast(&ServerMessage::FileOffered { offer: wire }, None);
        self.reporter.report(reported);
        info!(
            channel = channel.to_string(),
            user = user.to_string(),
            offer = id.to_string(),
            "file offered"
        );
        let rooms = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(u64::from(valid_for_seconds))).await;
            rooms.end_offer(channel, id, OfferEnd::Expired);
        });
        Ok(())
    }

    /// Takes back one of the participant's offers.
    pub fn withdraw_file(&self, seat: Seat, offer: Uuid) -> Result<(), RoomError> {
        let Seat { channel, user, .. } = seat;
        let room = self.room(channel)?;
        room.require(seat)?;
        let own = room
            .offers
            .lock()
            .expect("offers lock")
            .get(&offer)
            .is_some_and(|o| o.from == user);
        if !own {
            return Err(RoomError::UnknownOffer);
        }
        self.end_offer(channel, offer, OfferEnd::Withdrawn);
        Ok(())
    }

    /// Withdraws every offer `user` has standing in `channel`'s call, as when they may no
    /// longer offer files.
    pub(super) fn withdraw_offers_of(&self, channel: Uuid, user: Uuid) {
        let Ok(room) = self.room(channel) else {
            return;
        };
        let own: Vec<Uuid> = room
            .offers
            .lock()
            .expect("offers lock")
            .iter()
            .filter(|(_, offer)| offer.from == user)
            .map(|(id, _)| *id)
            .collect();
        for offer in own {
            self.end_offer(channel, offer, OfferEnd::NotPermitted);
        }
    }

    /// Ends an offer, if it still stands, telling everyone.
    fn end_offer(&self, channel: Uuid, offer: Uuid, reason: OfferEnd) {
        let Ok(room) = self.room(channel) else {
            return;
        };
        if room
            .offers
            .lock()
            .expect("offers lock")
            .remove(&offer)
            .is_some()
        {
            room.broadcast(&ServerMessage::FileWithdrawn { offer, reason }, None);
        }
    }

    /// Starts a transfer of `offer` to the participant, in `mode`.
    pub async fn accept_file(
        &self,
        seat: Seat,
        offer: Uuid,
        mode: TransferMode,
    ) -> Result<(), RoomError> {
        let Seat { channel, user, .. } = seat;
        let room = self.room(channel)?;
        let (sender, allow_direct, name, size, record) = {
            let offers = room.offers.lock().expect("offers lock");
            let standing = offers
                .get(&offer)
                .filter(|o| o.expires > Instant::now())
                .ok_or(RoomError::UnknownOffer)?;
            (
                standing.from,
                standing.allow_direct,
                standing.name.clone(),
                standing.size,
                standing.record,
            )
        };
        if sender == user {
            return Err(RoomError::BadParameters(
                "an offer is for the others in the call".to_string(),
            ));
        }
        match mode {
            TransferMode::DirectPreferred if !allow_direct => {
                return Err(RoomError::BadParameters(
                    "the sender allows only relayed transfers of this file".to_string(),
                ));
            }
            TransferMode::RelayOnly if self.relay.policy().relay_mbps.is_none() => {
                return Err(RoomError::BadParameters(
                    "this server does not relay transfers".to_string(),
                ));
            }
            _ => {}
        }
        if room.transfers_of(user) >= MAX_TRANSFERS || room.transfers_of(sender) >= MAX_TRANSFERS {
            return Err(RoomError::BadParameters(format!(
                "a participant may be part of at most {MAX_TRANSFERS} transfers at once"
            )));
        }
        {
            // Both sides are checked, and the transfer made, under the room's lock, so it is
            // made before either leaves, and ended as they do, or refused.
            let participants = room.participants.lock().expect("room lock");
            seated(&participants, seat)?;
            if !participants.contains_key(&sender) {
                return Err(RoomError::UnknownOffer);
            }
            let mut transfers = room.transfers.lock().expect("transfers lock");
            if transfers.contains_key(&(offer, user)) {
                return Err(RoomError::BadParameters(
                    "that file is already on its way to you".to_string(),
                ));
            }
            transfers.insert(
                (offer, user),
                Transfer {
                    sender,
                    mode,
                    record,
                },
            );
        }
        metrics::gauge!(aspen_metrics::voice::TRANSFERS, "mode" => mode_label(mode)).increment(1.0);
        let starting = |peer: Uuid, role: TransferRole| ServerMessage::TransferStarting {
            offer,
            peer,
            role,
            mode,
            name: name.clone(),
            size,
            ice_servers: self.relay.open(record, user, role, mode),
        };
        room.send_to(sender, starting(user, TransferRole::Sender));
        room.send_to(user, starting(sender, TransferRole::Receiver));
        room.broadcast(
            &ServerMessage::TransferLinkChanged {
                link: TransferLink {
                    sender,
                    receiver: user,
                },
                active: true,
            },
            None,
        );
        self.reporter.report(VoiceReport::TransferStarted {
            channel,
            record,
            sender,
            receiver: user,
            mode,
        });
        info!(
            channel = channel.to_string(),
            sender = sender.to_string(),
            receiver = user.to_string(),
            offer = offer.to_string(),
            mode = mode_label(mode),
            "transfer started"
        );
        Ok(())
    }

    /// Passes part of a transfer's peer connection from the participant to `peer`.
    pub fn transfer_signal(
        &self,
        seat: Seat,
        offer: Uuid,
        peer: Uuid,
        signal: Value,
    ) -> Result<(), RoomError> {
        let size = serde_json::to_vec(&signal).map_or(usize::MAX, |json| json.len());
        if size > MAX_SIGNAL_BYTES {
            return Err(RoomError::BadParameters(format!(
                "a transfer signal may be at most {MAX_SIGNAL_BYTES} bytes"
            )));
        }
        let user = seat.user;
        let room = self.room(seat.channel)?;
        room.require(seat)?;
        if room.transfer_between(offer, user, peer).is_none() {
            return Err(RoomError::UnknownTransfer);
        }
        room.send_to(
            peer,
            ServerMessage::TransferSignal {
                offer,
                peer: user,
                signal,
            },
        );
        Ok(())
    }

    /// Ends the transfer of `offer` between the participant and `peer`, at once, for `reason`.
    pub async fn end_transfer(
        &self,
        seat: Seat,
        offer: Uuid,
        peer: Uuid,
        reason: TransferEnd,
    ) -> Result<(), RoomError> {
        if reason == TransferEnd::Left {
            return Err(RoomError::BadParameters(
                "only the server ends a transfer for someone leaving".to_string(),
            ));
        }
        let user = seat.user;
        let room = self.room(seat.channel)?;
        room.require(seat)?;
        let (key, _) = room
            .transfer_between(offer, user, peer)
            .ok_or(RoomError::UnknownTransfer)?;
        self.finish(&room, key, user, reason).await;
        Ok(())
    }

    /// Removes one transfer: closes both its sides at the relay, tells the side that did not
    /// end it (`ended_by`), and tells everyone the link is gone.
    async fn finish(&self, room: &Room, key: (Uuid, Uuid), ended_by: Uuid, reason: TransferEnd) {
        let Some(transfer) = room.transfers.lock().expect("transfers lock").remove(&key) else {
            return;
        };
        let (offer, receiver) = key;
        let sender = transfer.sender;
        for role in [TransferRole::Sender, TransferRole::Receiver] {
            self.relay.close(transfer.record, receiver, role).await;
        }
        metrics::gauge!(aspen_metrics::voice::TRANSFERS, "mode" => mode_label(transfer.mode))
            .decrement(1.0);
        let other = if ended_by == sender { receiver } else { sender };
        room.send_to(
            other,
            ServerMessage::TransferEnded {
                offer,
                peer: ended_by,
                reason,
            },
        );
        room.broadcast(
            &ServerMessage::TransferLinkChanged {
                link: TransferLink { sender, receiver },
                active: false,
            },
            None,
        );
        self.reporter.report(VoiceReport::TransferEnded {
            channel: room.channel,
            record: transfer.record,
            receiver,
            ended_by,
            reason,
        });
    }

    /// Everything of `user`'s in a call they are leaving: their offers are withdrawn and every
    /// transfer they are part of ends.
    pub(super) async fn end_everything_of(&self, room: &Room, user: Uuid) {
        let offers: Vec<Uuid> = room
            .offers
            .lock()
            .expect("offers lock")
            .iter()
            .filter(|(_, offer)| offer.from == user)
            .map(|(id, _)| *id)
            .collect();
        for offer in offers {
            if room
                .offers
                .lock()
                .expect("offers lock")
                .remove(&offer)
                .is_some()
            {
                room.broadcast(
                    &ServerMessage::FileWithdrawn {
                        offer,
                        reason: OfferEnd::Left,
                    },
                    None,
                );
            }
        }
        let keys: Vec<(Uuid, Uuid)> = room
            .transfers
            .lock()
            .expect("transfers lock")
            .iter()
            .filter(|((_, receiver), transfer)| *receiver == user || transfer.sender == user)
            .map(|(key, _)| *key)
            .collect();
        for key in keys {
            self.finish(room, key, user, TransferEnd::Left).await;
        }
    }
}
