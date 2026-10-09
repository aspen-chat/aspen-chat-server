import type { SyncStatus } from "@aspen/protocol";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  DISCONNECTED_GRACE_MS,
  watchConnectionSounds,
  type ConnectionSoundSource,
} from "./connectionSounds";
import type { Sound } from "./sounds";

function watched() {
  let status: SyncStatus = "live";
  const listeners = new Set<() => void>();
  const sync: ConnectionSoundSource = {
    get status() {
      return status;
    },
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
  const played: Sound[] = [];
  const stop = watchConnectionSounds(sync, (sound) => played.push(sound));
  return {
    played,
    stop,
    set(next: SyncStatus) {
      status = next;
      for (const listener of listeners) {
        listener();
      }
    },
  };
}

describe("watchConnectionSounds", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("plays once when the stream stays down past the grace", () => {
    const connection = watched();
    connection.set("reconnecting");
    vi.advanceTimersByTime(DISCONNECTED_GRACE_MS - 1);
    expect(connection.played).toEqual([]);
    vi.advanceTimersByTime(1);
    vi.advanceTimersByTime(DISCONNECTED_GRACE_MS * 10);
    expect(connection.played).toEqual(["disconnected"]);
  });

  it("is silent when the stream comes back in time, or the user signs out", () => {
    const connection = watched();
    connection.set("reconnecting");
    connection.set("resyncing");
    connection.set("live");
    connection.set("reconnecting");
    connection.set("stopped");
    vi.advanceTimersByTime(DISCONNECTED_GRACE_MS * 2);
    expect(connection.played).toEqual([]);
  });

  it("is silent once stopped", () => {
    const connection = watched();
    connection.set("reconnecting");
    connection.stop();
    vi.advanceTimersByTime(DISCONNECTED_GRACE_MS * 2);
    expect(connection.played).toEqual([]);
  });
});
