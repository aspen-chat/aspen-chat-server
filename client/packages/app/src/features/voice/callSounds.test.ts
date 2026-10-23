import {
  NO_FILES,
  type ChannelVoice,
  type FilesState,
  type TransferState,
  type User,
  type VoiceCallState,
  type VoiceParticipantState,
} from "@aspen/protocol";
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
    sharingScreen: false,
    files: NO_FILES,
  } as Partial<VoiceCallState> as VoiceCallState;
  const callListeners = new Set<() => void>();
  const topicListeners = new Map<string, Set<() => void>>();
  const participants = new Map<string, string[]>();
  const sharers = new Set<string>();
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
          (user) =>
            ({
              user,
              sharingScreen: sharers.has(user),
            }) as Partial<VoiceParticipantState> as VoiceParticipantState,
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
    setParticipants(channelId: string, users: string[], sharing: string[] = []) {
      participants.set(channelId, users);
      sharers.clear();
      for (const user of sharing) {
        sharers.add(user);
      }
      for (const listener of topicListeners.get(`voice:${channelId}`) ?? []) {
        listener();
      }
    },
    setFiles(patch: Partial<FilesState>) {
      this.setCall({ files: { ...state.files, ...patch } });
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

  it("says a lost call is lost, and back when it rejoins", () => {
    const call = watched();
    call.setCall({ status: "connected", channelId: "c" });
    call.setCall({ status: "rejoining" });
    call.setCall({ status: "connected" });
    call.setCall({ status: "idle", channelId: null, endedReason: "kicked" });
    expect(call.played).toEqual(["callJoined", "disconnected", "reconnected", "disconnected"]);
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

  it("plays a failed rejoin's loss once, and a fresh join after it as joining", () => {
    const call = watched();
    call.setCall({ status: "connected", channelId: "c" });
    call.setCall({ status: "rejoining" });
    call.setCall({ status: "failed" });
    call.setCall({ status: "joining" });
    call.setCall({ status: "connected" });
    expect(call.played).toEqual(["callJoined", "disconnected", "callJoined"]);
  });

  it("plays a share starting, the user's own or another's, but not one already under way", () => {
    const call = watched();
    call.setParticipants("c", ["a", "b"], ["a"]);
    call.setCall({ status: "connected", channelId: "c" });
    call.setParticipants("c", ["a", "b"], ["a", "b"]);
    call.setParticipants("c", ["a", "b"], ["a"]);
    call.setCall({ sharingScreen: true });
    call.setCall({ sharingScreen: false });
    expect(call.played).toEqual(["callJoined", "streamStarted", "streamStarted"]);
  });

  it("is silent about others' shares while deafened, but not about the user's own", () => {
    const call = watched();
    call.setCall({ status: "connected", channelId: "c", deafened: true });
    call.setParticipants("c", [], []);
    call.setParticipants("c", ["a"], ["a"]);
    call.setCall({ sharingScreen: true });
    expect(call.played).toEqual(["callJoined", "streamStarted"]);
  });

  it("plays every transfer beginning in the call, but not those under way when it connects", () => {
    const call = watched();
    call.setCall({ status: "joining", channelId: "c" });
    call.setFiles({ links: [{ sender: "a", receiver: "b" }] });
    call.setCall({ status: "connected" });
    call.setFiles({
      links: [
        { sender: "a", receiver: "b" },
        { sender: "a", receiver: "b" },
      ],
    });
    call.setFiles({ links: [{ sender: "a", receiver: "b" }] });
    call.setFiles({ links: [] });
    call.setFiles({ links: [{ sender: "me", receiver: "d" }] });
    expect(call.played).toEqual(["callJoined", "transferAccepted", "transferAccepted"]);
  });

  it("plays only the user's own transfers beginning while deafened", () => {
    const call = watched();
    call.setCall({ status: "connected", channelId: "c", deafened: true });
    call.setFiles({ links: [{ sender: "a", receiver: "b" }] });
    call.setFiles({
      links: [
        { sender: "a", receiver: "b" },
        { sender: "a", receiver: "me" },
      ],
    });
    expect(call.played).toEqual(["callJoined", "transferAccepted"]);
  });

  it("plays a transfer arriving whole, sent or received, once", () => {
    const call = watched();
    call.setCall({ status: "connected", channelId: "c" });
    const sent = transfer("o1", "a", "sender", "moving");
    const received = transfer("o2", "b", "receiver", "moving");
    call.setFiles({ transfers: [sent, received] });
    call.setFiles({ transfers: [{ ...sent, status: "completed" }, received] });
    call.setFiles({
      transfers: [
        { ...sent, status: "completed" },
        { ...received, status: "failed" },
      ],
    });
    call.setFiles({ transfers: [{ ...received, status: "failed" }] });
    call.setFiles({ transfers: [{ ...received, status: "completed" }] });
    expect(call.played).toEqual(["callJoined", "transferComplete", "transferComplete"]);
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

function transfer(
  offer: string,
  peer: string,
  role: TransferState["role"],
  status: TransferState["status"],
): TransferState {
  return { offer, peer, role, status } as Partial<TransferState> as TransferState;
}
