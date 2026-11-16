//! The call's sounds, as cues the platform plays through the call's own audio path: each is
//! derived from one change of the published `CallState` to the next, so a snapshot read again or
//! a frame that changes nothing gives none, and the set is the web client's (`callSounds.ts`
//! derives the same seven from `VoiceCallState` and the store), decided by the same rules:
//!
//! - `CallJoined` when the user's call connects, and when someone else joins it;
//! - `CallLeft` when the user leaves (another of their clients taking the call over counts), and
//!   when someone else leaves it;
//! - `Disconnected` when the call is lost under them: a lost server or a dropped socket the
//!   server would not give back (which the engine rejoins; one taken back with `resume` within
//!   the grace keeps the call `Connected` and is no sound, as in the web client), a fatal
//!   error, or an end the call bar explains (removed, access lost, idle);
//! - `Reconnected` when a call lost that way is connected again;
//! - `StreamStarted` when the user, or someone else in the call, starts sharing a screen;
//! - `TransferAccepted` and `TransferComplete` are given by nothing: the snapshot carries no file
//!   transfers (a native call's `canTransfer` is false), so there is nothing to derive them from.
//!
//! Moving to another call gives only the joining. Others' comings and goings and their shares are
//! not given while the user is deafened, and are compared only while the call is connected: who
//! is there as it connects, and what they share, is taken as found, silently, and the user's own
//! participant, should the server ever list it, is no one. The call stays connected through a
//! resume, so whoever a resume's `ready` lists come or gone since is heard as any joining or
//! leaving, as the web client hears them from its event stream while its socket is down. Each
//! cue is given again only after its `repeat_gap`, as the web client's `playSound` plays one:
//! several people joining at once are heard as one, and a call lost twice within half a minute
//! is said lost once.
//!
//! The platform decides how a cue sounds: which file, that at most two of one cue play at once
//! (the web client's `MAX_PLAYING_PER_SOUND`), and what becomes of one that arrives before the
//! call's audio session is active.

use crate::state::{CallState, CallStatus};
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;
use tokio::time::Instant;
use uuid::Uuid;
use voice_protocol::signal::MediaSource;

/// One of the call's sounds, named as the web client's `Sound` names the same recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum SoundCue {
    CallJoined,
    CallLeft,
    Disconnected,
    /// Back after `Disconnected` was given.
    Reconnected,
    StreamStarted,
    /// Someone accepted a file offered in the call, and it has begun to move.
    TransferAccepted,
    /// A file the user sent or received has arrived whole.
    TransferComplete,
}

impl SoundCue {
    /// How soon the cue may be given again; one within it is dropped. The web client's
    /// `REPEAT_GAP_MS`: a burst of joins, leaves, or accepted offers is heard once, and a call
    /// dropped and back twice within half a minute is said lost, and back, once.
    pub fn repeat_gap(self) -> Duration {
        match self {
            SoundCue::CallJoined
            | SoundCue::CallLeft
            | SoundCue::TransferAccepted
            | SoundCue::TransferComplete => Duration::from_millis(250),
            SoundCue::Disconnected | SoundCue::Reconnected => Duration::from_secs(30),
            SoundCue::StreamStarted => Duration::ZERO,
        }
    }
}

/// What is compared from one snapshot to the next, and when each cue was last given.
#[derive(Debug, Default)]
pub struct Sounds {
    /// The others in the call as last compared, while it is connected.
    present: BTreeSet<Uuid>,
    /// Those of them sharing a screen as last compared.
    sharing: BTreeSet<Uuid>,
    /// Whether `Disconnected` was given for a call that is rejoining.
    lost: bool,
    last_given: HashMap<SoundCue, Instant>,
}

impl Sounds {
    /// The cues for the change from `before` to `after`, in the order they are played, each
    /// kind at most once and none within its `repeat_gap` of the last.
    pub fn cues(&mut self, before: &CallState, after: &CallState) -> Vec<SoundCue> {
        let mut cues = Vec::new();
        let was_in = before.status == CallStatus::Connected;
        let is_in = after.status == CallStatus::Connected;
        // A move to another call passes through `Joining`, so a connected call never changes
        // its channel in one step.
        if is_in && !was_in {
            self.present = others(after);
            self.sharing = sharers(after);
            // `lost` stands only while the call is still `Rejoining`, so this is a rejoin's end.
            let rejoined = self.lost;
            self.lost = false;
            cues.push(if rejoined {
                SoundCue::Reconnected
            } else {
                SoundCue::CallJoined
            });
        } else if was_in && !is_in {
            if after.status == CallStatus::Idle && after.ended_reason.is_none() {
                cues.push(SoundCue::CallLeft);
            } else if after.status != CallStatus::Joining {
                self.lost = after.status == CallStatus::Rejoining;
                cues.push(SoundCue::Disconnected);
            }
        } else if was_in && is_in {
            let present = others(after);
            let sharing = sharers(after);
            let joined = present.difference(&self.present).next().is_some();
            let left = self.present.difference(&present).next().is_some();
            let shared = sharing.difference(&self.sharing).next().is_some();
            self.present = present;
            self.sharing = sharing;
            if !after.deafened {
                if joined {
                    cues.push(SoundCue::CallJoined);
                }
                if left {
                    cues.push(SoundCue::CallLeft);
                }
                if shared {
                    cues.push(SoundCue::StreamStarted);
                }
            }
        } else if after.status != CallStatus::Rejoining {
            self.lost = false;
        }
        // A screen is asked for only in a connected call (`start_source`), and the call's end
        // forgets every source, so this is the user's own share starting.
        if !sends_screen(before) && sends_screen(after) {
            cues.push(SoundCue::StreamStarted);
        }
        let now = Instant::now();
        cues.retain(|&cue| {
            let recent = self
                .last_given
                .get(&cue)
                .is_some_and(|last| now.duration_since(*last) < cue.repeat_gap());
            if !recent {
                self.last_given.insert(cue, now);
            }
            !recent
        });
        cues
    }
}

/// Everyone in the call but the user.
fn others(state: &CallState) -> BTreeSet<Uuid> {
    state
        .participants
        .iter()
        .map(|participant| participant.user)
        .filter(|user| Some(*user) != state.user)
        .collect()
}

/// Those of the others sharing a screen.
fn sharers(state: &CallState) -> BTreeSet<Uuid> {
    state
        .participants
        .iter()
        .filter(|participant| Some(participant.user) != state.user)
        .filter(|participant| {
            participant
                .sources
                .iter()
                .any(|source| source.source == MediaSource::Screen)
        })
        .map(|participant| participant.user)
        .collect()
}

/// Whether the user sends, or has asked to send, a screen.
fn sends_screen(state: &CallState) -> bool {
    state
        .sources
        .iter()
        .any(|source| source.source == MediaSource::Screen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer_connection::TrackId;
    use crate::state::{EndedReason, LocalSource, Participant};
    use voice_protocol::signal::SourceInfo;

    fn participant(user: Uuid, sources: &[MediaSource]) -> Participant {
        Participant {
            user,
            muted: false,
            deafened: false,
            held: false,
            speaking: false,
            sources: sources
                .iter()
                .map(|&source| SourceInfo {
                    source,
                    paused: false,
                })
                .collect(),
        }
    }

    fn connected(me: Uuid, channel: Uuid, participants: Vec<Participant>) -> CallState {
        CallState {
            status: CallStatus::Connected,
            channel: Some(channel),
            user: Some(me),
            participants,
            ..CallState::default()
        }
    }

    fn with_status(state: &CallState, status: CallStatus) -> CallState {
        CallState {
            status,
            ..state.clone()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_repeat_gaps_are_the_web_clients() {
        assert_eq!(
            SoundCue::CallJoined.repeat_gap(),
            Duration::from_millis(250)
        );
        assert_eq!(SoundCue::CallLeft.repeat_gap(), Duration::from_millis(250));
        assert_eq!(SoundCue::Disconnected.repeat_gap(), Duration::from_secs(30));
        assert_eq!(SoundCue::Reconnected.repeat_gap(), Duration::from_secs(30));
        assert_eq!(SoundCue::StreamStarted.repeat_gap(), Duration::ZERO);
        assert_eq!(
            SoundCue::TransferAccepted.repeat_gap(),
            Duration::from_millis(250)
        );
        assert_eq!(
            SoundCue::TransferComplete.repeat_gap(),
            Duration::from_millis(250)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn who_is_there_at_the_connection_is_taken_as_found() {
        let (me, channel, a, b) = (
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        let mut sounds = Sounds::default();
        let idle = CallState::default();
        let joining = CallState {
            status: CallStatus::Joining,
            channel: Some(channel),
            participants: vec![participant(a, &[]), participant(b, &[MediaSource::Screen])],
            ..CallState::default()
        };
        assert_eq!(sounds.cues(&idle, &joining), vec![]);
        let in_call = connected(me, channel, joining.participants.clone());
        assert_eq!(sounds.cues(&joining, &in_call), vec![SoundCue::CallJoined]);
        tokio::time::advance(Duration::from_secs(1)).await;
        // The user's own participant, should the server list it, is no one joining or sharing.
        let mut with_me = in_call.clone();
        with_me
            .participants
            .push(participant(me, &[MediaSource::Screen]));
        assert_eq!(sounds.cues(&in_call, &with_me), vec![]);
        assert_eq!(sounds.cues(&with_me, &in_call), vec![]);
    }

    #[tokio::test(start_paused = true)]
    async fn one_leaving_as_another_joins_is_both() {
        let (me, channel, a, b) = (
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        let mut sounds = Sounds::default();
        let with_a = connected(me, channel, vec![participant(a, &[])]);
        sounds.cues(&CallState::default(), &with_a);
        tokio::time::advance(Duration::from_secs(1)).await;
        let with_b = connected(me, channel, vec![participant(b, &[])]);
        assert_eq!(
            sounds.cues(&with_a, &with_b),
            vec![SoundCue::CallJoined, SoundCue::CallLeft]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn others_coming_going_and_sharing_are_given_once_each_and_not_while_deafened() {
        let (me, channel, a, b) = (
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        let mut sounds = Sounds::default();
        let alone = connected(me, channel, Vec::new());
        assert_eq!(
            sounds.cues(&CallState::default(), &alone),
            vec![SoundCue::CallJoined]
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        let two = connected(me, channel, vec![participant(a, &[]), participant(b, &[])]);
        assert_eq!(sounds.cues(&alone, &two), vec![SoundCue::CallJoined]);
        tokio::time::advance(Duration::from_secs(1)).await;
        // A joins again from another device while B shares: no one new, one new share.
        let rejoined = connected(
            me,
            channel,
            vec![participant(b, &[MediaSource::Screen]), participant(a, &[])],
        );
        assert_eq!(sounds.cues(&two, &rejoined), vec![SoundCue::StreamStarted]);
        // A share that goes on, and a camera, give nothing.
        let camera = connected(
            me,
            channel,
            vec![
                participant(b, &[MediaSource::Screen, MediaSource::Camera]),
                participant(a, &[MediaSource::Camera]),
            ],
        );
        assert_eq!(sounds.cues(&rejoined, &camera), vec![]);
        tokio::time::advance(Duration::from_secs(1)).await;
        let mut deafened = connected(me, channel, vec![participant(a, &[])]);
        deafened.deafened = true;
        assert_eq!(sounds.cues(&camera, &deafened), vec![]);
        let mut back = connected(
            me,
            channel,
            vec![participant(a, &[]), participant(b, &[MediaSource::Screen])],
        );
        back.deafened = true;
        assert_eq!(sounds.cues(&deafened, &back), vec![]);
        // Undeafened: what changed while deafened was still taken in, so nothing is said of it.
        let heard = connected(me, channel, back.participants.clone());
        assert_eq!(sounds.cues(&back, &heard), vec![]);
        tokio::time::advance(Duration::from_secs(1)).await;
        let gone = connected(me, channel, Vec::new());
        assert_eq!(sounds.cues(&heard, &gone), vec![SoundCue::CallLeft]);
    }

    #[tokio::test(start_paused = true)]
    async fn leaving_is_left_and_being_ended_is_disconnected() {
        let (me, channel) = (Uuid::now_v7(), Uuid::now_v7());
        let in_call = connected(me, channel, Vec::new());
        for (ended, cue) in [
            (None, SoundCue::CallLeft),
            (Some(EndedReason::Kicked), SoundCue::Disconnected),
            (Some(EndedReason::AccessLost), SoundCue::Disconnected),
            (Some(EndedReason::Idle), SoundCue::Disconnected),
        ] {
            let mut sounds = Sounds::default();
            let idle = CallState {
                ended_reason: ended,
                ..CallState::default()
            };
            assert_eq!(sounds.cues(&in_call, &idle), vec![cue], "{ended:?}");
            assert!(!sounds.lost);
        }
        let mut sounds = Sounds::default();
        let failed = with_status(&in_call, CallStatus::Failed);
        assert_eq!(sounds.cues(&in_call, &failed), vec![SoundCue::Disconnected]);
        assert!(!sounds.lost);
    }

    #[tokio::test(start_paused = true)]
    async fn a_lost_call_is_said_lost_once_and_back_once() {
        let (me, channel, a) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let mut sounds = Sounds::default();
        let in_call = connected(me, channel, vec![participant(a, &[])]);
        assert_eq!(
            sounds.cues(&CallState::default(), &in_call),
            vec![SoundCue::CallJoined]
        );
        let rejoining = with_status(&in_call, CallStatus::Rejoining);
        assert_eq!(
            sounds.cues(&in_call, &rejoining),
            vec![SoundCue::Disconnected]
        );
        assert!(sounds.lost);
        // A resume's `ready` restates the participants, one of them gone meanwhile: silently.
        let back = connected(me, channel, Vec::new());
        assert_eq!(sounds.cues(&rejoining, &back), vec![SoundCue::Reconnected]);
        assert!(!sounds.lost);
        // Lost again within the gap: neither is said again.
        tokio::time::advance(Duration::from_secs(29)).await;
        let rejoining = with_status(&back, CallStatus::Rejoining);
        assert_eq!(sounds.cues(&back, &rejoining), vec![]);
        assert!(sounds.lost, "the loss is remembered though not said");
        assert_eq!(sounds.cues(&rejoining, &back), vec![]);
        // Past the gap, both are said.
        tokio::time::advance(Duration::from_secs(30)).await;
        assert_eq!(sounds.cues(&back, &rejoining), vec![SoundCue::Disconnected]);
        tokio::time::advance(Duration::from_secs(30)).await;
        assert_eq!(sounds.cues(&rejoining, &back), vec![SoundCue::Reconnected]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_rejoin_that_fails_adds_nothing_and_a_fresh_join_after_is_a_joining() {
        let (me, channel) = (Uuid::now_v7(), Uuid::now_v7());
        let mut sounds = Sounds::default();
        let in_call = connected(me, channel, Vec::new());
        sounds.cues(&CallState::default(), &in_call);
        let rejoining = with_status(&in_call, CallStatus::Rejoining);
        assert_eq!(
            sounds.cues(&in_call, &rejoining),
            vec![SoundCue::Disconnected]
        );
        let failed = with_status(&in_call, CallStatus::Failed);
        assert_eq!(sounds.cues(&rejoining, &failed), vec![]);
        assert!(!sounds.lost);
        let joining = with_status(&in_call, CallStatus::Joining);
        assert_eq!(sounds.cues(&failed, &joining), vec![]);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(sounds.cues(&joining, &in_call), vec![SoundCue::CallJoined]);
    }

    #[tokio::test(start_paused = true)]
    async fn moving_to_another_call_gives_only_the_joining() {
        let (me, first, second) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let mut sounds = Sounds::default();
        let in_first = connected(me, first, Vec::new());
        sounds.cues(&CallState::default(), &in_first);
        tokio::time::advance(Duration::from_secs(1)).await;
        let joining = CallState {
            status: CallStatus::Joining,
            channel: Some(second),
            ..CallState::default()
        };
        assert_eq!(sounds.cues(&in_first, &joining), vec![]);
        let in_second = connected(me, second, Vec::new());
        assert_eq!(
            sounds.cues(&joining, &in_second),
            vec![SoundCue::CallJoined]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_users_own_share_is_given_as_it_is_asked() {
        let (me, channel) = (Uuid::now_v7(), Uuid::now_v7());
        let mut sounds = Sounds::default();
        let in_call = connected(me, channel, Vec::new());
        sounds.cues(&CallState::default(), &in_call);
        let mut sharing = in_call.clone();
        sharing.sources.push(LocalSource {
            source: MediaSource::Screen,
            track: TrackId("screen".to_string()),
            live: false,
        });
        assert_eq!(
            sounds.cues(&in_call, &sharing),
            vec![SoundCue::StreamStarted]
        );
        let mut live = sharing.clone();
        live.sources[0].live = true;
        live.sources.push(LocalSource {
            source: MediaSource::ScreenAudio,
            track: TrackId("screen-audio".to_string()),
            live: false,
        });
        assert_eq!(sounds.cues(&sharing, &live), vec![]);
        // Stopping says nothing; sharing again does.
        assert_eq!(sounds.cues(&live, &in_call), vec![]);
        assert_eq!(
            sounds.cues(&in_call, &sharing),
            vec![SoundCue::StreamStarted]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn joins_within_the_gap_are_heard_once() {
        let (me, channel, a, b) = (
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        let mut sounds = Sounds::default();
        let alone = connected(me, channel, Vec::new());
        sounds.cues(&CallState::default(), &alone);
        tokio::time::advance(Duration::from_millis(250)).await;
        let one = connected(me, channel, vec![participant(a, &[])]);
        assert_eq!(sounds.cues(&alone, &one), vec![SoundCue::CallJoined]);
        tokio::time::advance(Duration::from_millis(249)).await;
        let two = connected(me, channel, vec![participant(a, &[]), participant(b, &[])]);
        assert_eq!(sounds.cues(&one, &two), vec![]);
        tokio::time::advance(Duration::from_millis(1)).await;
        // The gap counts from the last one given, not the last one asked.
        let three = connected(
            me,
            channel,
            vec![
                participant(a, &[]),
                participant(b, &[]),
                participant(Uuid::now_v7(), &[]),
            ],
        );
        assert_eq!(sounds.cues(&two, &three), vec![SoundCue::CallJoined]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_snapshot_that_changes_nothing_of_the_call_gives_nothing() {
        let (me, channel, a) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let mut sounds = Sounds::default();
        let in_call = connected(me, channel, vec![participant(a, &[])]);
        sounds.cues(&CallState::default(), &in_call);
        let mut muted = in_call.clone();
        muted.muted = true;
        muted.participants[0].speaking = true;
        muted.revision = 7;
        assert_eq!(sounds.cues(&in_call, &muted), vec![]);
        assert_eq!(sounds.cues(&muted, &muted), vec![]);
    }
}
