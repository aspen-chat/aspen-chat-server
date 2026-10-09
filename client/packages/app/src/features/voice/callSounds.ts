import type { AspenSync } from "@aspen/protocol";
import type { Sound } from "@/features/notifications/sounds";

/** What `watchCallSounds` reads of a deployment's sync. */
export interface CallSoundSource {
  readonly voice: Pick<AspenSync["voice"], "state" | "subscribe">;
  readonly store: Pick<AspenSync["store"], "subscribe" | "channelVoice" | "me">;
}

/**
 * Plays the call's sounds on one deployment until the function it answers is called:
 *
 * - `callJoined` when the user's call connects, or connects again after a rejoin, and when
 *   someone else joins it;
 * - `callLeft` when the user leaves (or another of their clients takes the call over), and when
 *   someone else leaves it;
 * - `disconnected` when the call is lost under them: a lost voice server or socket (which
 *   rejoins, and so is followed by `callJoined`), a failed rejoin, or an end the call bar
 *   explains (idle, removed, access lost).
 *
 * Moving to another call plays only the joining. Others' comings and goings are not played
 * while the user is deafened, and are compared only while the call is connected: who is there
 * when it connects is taken as it finds them, silently.
 */
export function watchCallSounds(sync: CallSoundSource, play: (sound: Sound) => void): () => void {
  let state = sync.voice.state;
  let present = new Set<string>();
  let stopParticipants: (() => void) | null = null;

  const othersIn = (channelId: string): Set<string> => {
    const me = sync.store.me()?.id;
    return new Set(
      sync.store
        .channelVoice(channelId)
        .participants.map((participant) => participant.user)
        .filter((user) => user !== me),
    );
  };

  const onParticipants = () => {
    if (state.status !== "connected" || state.channelId === null) {
      return;
    }
    const now = othersIn(state.channelId);
    const joined = [...now].some((user) => !present.has(user));
    const left = [...present].some((user) => !now.has(user));
    present = now;
    if (state.deafened) {
      return;
    }
    if (joined) {
      play("callJoined");
    }
    if (left) {
      play("callLeft");
    }
  };

  const watch = (channelId: string | null) => {
    stopParticipants?.();
    stopParticipants = null;
    if (channelId !== null) {
      present = othersIn(channelId);
      stopParticipants = sync.store.subscribe(`voice:${channelId}`, onParticipants);
    }
  };

  const onCall = () => {
    const before = state;
    state = sync.voice.state;
    const wasIn = before.status === "connected";
    const isIn = state.status === "connected";
    if (isIn && (!wasIn || before.channelId !== state.channelId)) {
      watch(state.channelId);
      play("callJoined");
    } else if (wasIn && !isIn) {
      watch(null);
      if (state.status === "idle" && state.endedReason === null) {
        play("callLeft");
      } else if (state.status !== "joining") {
        play("disconnected");
      }
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
