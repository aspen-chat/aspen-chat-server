import type {
  AspenSync,
  TransferLink,
  TransferState,
  VoiceCallState,
  VoiceParticipantState,
} from "@aspen/protocol";
import type { Sound } from "@/features/notifications/sounds";

/** What `watchCallSounds` reads of a deployment's sync. */
export interface CallSoundSource {
  readonly voice: Pick<AspenSync["voice"], "state" | "subscribe">;
  readonly store: Pick<AspenSync["store"], "subscribe" | "channelVoice" | "me">;
}

/**
 * Plays the call's sounds on one deployment until the function it answers is called:
 *
 * - `callJoined` when the user's call connects, and when someone else joins it;
 * - `callLeft` when the user leaves (or another of their clients takes the call over), and when
 *   someone else leaves it;
 * - `disconnected` when the call is lost under them: a lost voice server or socket (which
 *   rejoins), a failed rejoin, or an end the call bar explains (idle, removed, access lost);
 * - `reconnected` when a call lost that way rejoins;
 * - `streamStarted` when the user, or someone else in the call, starts sharing a screen, window,
 *   or game;
 * - `transferAccepted` when any transfer in the call begins, whoever its two sides are: everyone
 *   in the call hears an offer accepted;
 * - `transferComplete` when a file the user sent or received has arrived whole.
 *
 * Moving to another call plays only the joining. Others' comings and goings, their shares, and
 * transfers the user is not a side of are not played while the user is deafened, and are
 * compared only while the call is connected: what is there when it connects is taken as it
 * finds it, silently.
 */
export function watchCallSounds(sync: CallSoundSource, play: (sound: Sound) => void): () => void {
  let state = sync.voice.state;
  let present = new Set<string>();
  let sharing = new Set<string>();
  let links = linkCounts(state.files.links);
  let completed = completedTransfers(state.files.transfers);
  /** Whether `disconnected` was played for a call that is rejoining. */
  let lost = false;
  let stopParticipants: (() => void) | null = null;

  const others = (channelId: string) => {
    const me = sync.store.me()?.id;
    return sync.store.channelVoice(channelId).participants.filter((p) => p.user !== me);
  };
  const usersOf = (participants: readonly VoiceParticipantState[]) =>
    new Set(participants.map((participant) => participant.user));
  const sharersOf = (participants: readonly VoiceParticipantState[]) =>
    new Set(participants.filter((p) => p.sharingScreen).map((participant) => participant.user));

  const onParticipants = () => {
    if (state.status !== "connected" || state.channelId === null) {
      return;
    }
    const now = others(state.channelId);
    const nowPresent = usersOf(now);
    const nowSharing = sharersOf(now);
    const joined = [...nowPresent].some((user) => !present.has(user));
    const left = [...present].some((user) => !nowPresent.has(user));
    const shared = [...nowSharing].some((user) => !sharing.has(user));
    present = nowPresent;
    sharing = nowSharing;
    if (state.deafened) {
      return;
    }
    if (joined) {
      play("callJoined");
    }
    if (left) {
      play("callLeft");
    }
    if (shared) {
      play("streamStarted");
    }
  };

  const watch = (channelId: string | null) => {
    stopParticipants?.();
    stopParticipants = null;
    if (channelId !== null) {
      const now = others(channelId);
      present = usersOf(now);
      sharing = sharersOf(now);
      stopParticipants = sync.store.subscribe(`voice:${channelId}`, onParticipants);
    }
  };

  const onTransfers = (before: VoiceCallState) => {
    const me = sync.store.me()?.id;
    const nowLinks = linkCounts(state.files.links);
    if (before.status === "connected" && state.status === "connected") {
      const begun = [...nowLinks].filter(([pair, count]) => count > (links.get(pair) ?? 0));
      const heard = begun.some(([pair]) => !state.deafened || pairHas(pair, me));
      if (heard) {
        play("transferAccepted");
      }
    }
    links = nowLinks;
    const nowCompleted = completedTransfers(state.files.transfers);
    const arrived = [...nowCompleted].some((key) => !completed.has(key));
    completed = nowCompleted;
    if (arrived) {
      play("transferComplete");
    }
  };

  const onCall = () => {
    const before = state;
    state = sync.voice.state;
    const wasIn = before.status === "connected";
    const isIn = state.status === "connected";
    if (isIn && (!wasIn || before.channelId !== state.channelId)) {
      watch(state.channelId);
      const rejoined = lost && before.status === "rejoining";
      lost = false;
      play(rejoined ? "reconnected" : "callJoined");
    } else if (wasIn && !isIn) {
      watch(null);
      if (state.status === "idle" && state.endedReason === null) {
        play("callLeft");
      } else if (state.status !== "joining") {
        lost = state.status === "rejoining";
        play("disconnected");
      }
    } else if (!isIn && state.status !== "rejoining") {
      lost = false;
    }
    if (isIn && !before.sharingScreen && state.sharingScreen) {
      play("streamStarted");
    }
    if (before.files !== state.files) {
      onTransfers(before);
    }
  };

  if (state.status === "connected") {
    watch(state.channelId);
  }
  const stopCall = sync.voice.subscribe(onCall);
  return () => {
    stopCall();
    watch(null);
  };
}

/** How many transfers are under way between each sender and receiver, keyed `sender receiver`. */
function linkCounts(links: readonly TransferLink[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const link of links) {
    const pair = `${link.sender} ${link.receiver}`;
    counts.set(pair, (counts.get(pair) ?? 0) + 1);
  }
  return counts;
}

function pairHas(pair: string, user: string | undefined): boolean {
  return user !== undefined && pair.split(" ").includes(user);
}

/**
 * The transfers that arrived whole, keyed `offer peer`. A finished transfer stays in the call's
 * state until dismissed, so one is new to this set only as it completes.
 */
function completedTransfers(transfers: readonly TransferState[]): Set<string> {
  return new Set(
    transfers
      .filter((transfer) => transfer.status === "completed")
      .map((transfer) => `${transfer.offer} ${transfer.peer}`),
  );
}
