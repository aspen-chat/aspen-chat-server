import type { ChannelVoice, User, VoiceCallState, VoiceParticipantState } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import type { Sound } from "@/features/notifications/sounds";
import { watchCallSounds, type CallSoundSource } from "./callSounds";

/** A deployment with a call and a store just big enough for `watchCallSounds`. */
function fakeSync() {
  let state = {
    status: "idle",
    channelId: null,
    deafened: false,
    endedReason: null,
  } as Partial<VoiceCallState> as VoiceCallState;
  const callListeners = new Set<() => void>();
  const topicListeners = new Map<string, Set<() => void>>();
  const participants = new Map<string, string[]>();
  const sync: CallSoundSource = {
    voice: {
      get state() {
        return state;
      },
      subscribe: (listener) => {
        callListeners.add(listener);
        return () => callListeners.delete(listener);
      },
    },
    store: {
      subscribe: (topic: string, listener: () => void) => {
        const set = topicListeners.get(topic) ?? new Set();
        set.add(listener);
        topicListeners.set(topic, set);
        return () => set.delete(listener);
      },
      channelVoice: (channelId: string): ChannelVoice => ({
        session: null,
        rings: [],
        participants: (participants.get(channelId) ?? []).map(
          (user) => ({ user }) as Partial<VoiceParticipantState> as VoiceParticipantState,
        ),
      }),
      me: () => ({ id: "me" }) as User,
    },
  };
  return {
    sync,
    setCall(patch: Partial<VoiceCallState>) {
      state = { ...state, ...patch };
      for (const listener of callListeners) {
        listener();
      }
    },
    setParticipants(channelId: string, users: string[]) {
      participants.set(channelId, users);
      for (const listener of topicListeners.get(`voice:${channelId}`) ?? []) {
        listener();
      }
    },
  };
}

function watched() {
  const fake = fakeSync();
  const played: Sound[] = [];
  const stop = watchCallSounds(fake.sync, (sound) => played.push(sound));
  return { ...fake, played, stop };
}

describe("watchCallSounds", () => {
  it("plays joining and leaving for the user and for others, but not who was already there", () => {
    const call = watched();
    call.setParticipants("c", ["a", "b"]);
    call.setCall({ status: "joining", channelId: "c" });
    call.setCall({ status: "connected" });
    expect(call.played).toEqual(["callJoined"]);
    call.setParticipants("c", ["a", "b", "me"]);
    call.setParticipants("c", ["a", "b", "me", "d"]);
    call.setParticipants("c", ["b", "me", "d"]);
    call.setCall({ status: "idle", channelId: null });
    expect(call.played).toEqual(["callJoined", "callJoined", "callLeft", "callLeft"]);
    call.setParticipants("c", ["b", "d"]);
    expect(call.played).toHaveLength(4);
  });

  it("is silent about others while deafened", () => {
    const call = watched();
    call.setCall({ status: "connected", channelId: "c" });
    call.setCall({ deafened: true });
    call.setParticipants("c", ["a"]);
    call.setParticipants("c", []);
    expect(call.played).toEqual(["callJoined"]);
  });

  it("says a lost call is lost, and joined again when it rejoins", () => {
    const call = watched();
    call.setCall({ status: "connected", channelId: "c" });
    call.setCall({ status: "rejoining" });
    call.setCall({ status: "connected" });
    call.setCall({ status: "idle", channelId: null, endedReason: "kicked" });
    expect(call.played).toEqual(["callJoined", "disconnected", "callJoined", "disconnected"]);
  });

  it("plays only the joining when moving to another call", () => {
    const call = watched();
    call.setCall({ status: "connected", channelId: "c" });
    call.setCall({ status: "joining", channelId: "d" });
    call.setCall({ status: "connected" });
    expect(call.played).toEqual(["callJoined", "callJoined"]);
    call.setParticipants("c", ["a"]);
    call.setParticipants("d", ["a"]);
    expect(call.played).toEqual(["callJoined", "callJoined", "callJoined"]);
  });

  it("stops listening when stopped", () => {
    const call = watched();
    call.setCall({ status: "connected", channelId: "c" });
    call.stop();
    call.setParticipants("c", ["a"]);
    call.setCall({ status: "idle", channelId: null });
    expect(call.played).toEqual(["callJoined"]);
  });
});
