import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** Stands in for `AudioBufferSourceNode`, recording when it was started and what it feeds. */
class FakeSource extends EventTarget {
  buffer: AudioBuffer | null = null;
  loop = false;
  startedAt: number | null = null;
  input: unknown = null;
  connect(node: unknown): void {
    this.input = node;
  }
  disconnect(): void {
    this.input = null;
  }
  start(when: number): void {
    this.startedAt = when;
  }
  stop(): void {
    this.dispatchEvent(new Event("ended"));
  }
}

/**
 * Stands in for `AudioContext`. It starts suspended, as a browser's does, and `resume` starts it
 * unless `FakeContext.refuse` is set, when it never does, as before the page is interacted with.
 */
class FakeContext {
  static made: FakeContext[] = [];
  static refuse = false;
  state: AudioContextState = "suspended";
  currentTime = 0;
  destination = { kind: "destination" };
  sources: FakeSource[] = [];
  streams: { stream: { kind: "stream" } }[] = [];
  constructor(readonly options?: { sinkId?: string }) {
    FakeContext.made.push(this);
  }
  resume(): Promise<void> {
    if (FakeContext.refuse) {
      return new Promise(() => undefined);
    }
    this.state = "running";
    return Promise.resolve();
  }
  close(): Promise<void> {
    this.state = "closed";
    return Promise.resolve();
  }
  createBufferSource(): FakeSource {
    const source = new FakeSource();
    this.sources.push(source);
    return source;
  }
  createMediaStreamDestination() {
    const destination = { stream: { kind: "stream" as const } };
    this.streams.push(destination);
    return destination;
  }
}

/** A context that can be sent to a device itself, as Chromium's can. */
class ChoosingContext extends FakeContext {
  setSinkId(): Promise<void> {
    return Promise.resolve();
  }
}

/** Stands in for the `<audio>` element that plays a context's stream to a chosen device. */
class FakeElement {
  static made: FakeElement[] = [];
  srcObject: unknown = null;
  sinkId = "";
  playing = false;
  constructor() {
    FakeElement.made.push(this);
  }
  setSinkId(id: string): Promise<void> {
    this.sinkId = id;
    return Promise.resolve();
  }
  play(): Promise<void> {
    this.playing = true;
    return Promise.resolve();
  }
  pause(): void {
    this.playing = false;
  }
}

const desk = { id: "desk", label: "Desk speakers" };

const decode = () => Promise.resolve({} as AudioBuffer);

/** A fresh copy of the module, so its speakers do not carry between tests. */
async function speakers() {
  vi.resetModules();
  return import("./speakers");
}

function lastContext(): FakeContext {
  const context = FakeContext.made.at(-1);
  if (context === undefined) {
    throw new Error("No context was made");
  }
  return context;
}

function lastSource(): FakeSource {
  const source = lastContext().sources.at(-1);
  if (source === undefined) {
    throw new Error("Nothing was played");
  }
  return source;
}

describe("speakers", () => {
  beforeEach(() => {
    FakeContext.made = [];
    FakeContext.refuse = false;
    FakeElement.made = [];
    vi.stubGlobal("AudioContext", FakeContext);
    vi.stubGlobal("Audio", FakeElement);
    vi.stubGlobal("navigator", {
      mediaDevices: {
        enumerateDevices: () =>
          Promise.resolve([{ kind: "audiooutput", deviceId: desk.id, label: desk.label }]),
      },
    });
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    Reflect.deleteProperty(HTMLMediaElement.prototype, "setSinkId");
  });

  it("waits for a waking device before the first sound, and plays the next at once", async () => {
    const { play, WAKE_SECONDS } = await speakers();
    await play(decode, null);
    expect(lastSource().startedAt).toBe(WAKE_SECONDS);
    lastContext().currentTime = 1;
    await play(decode, null);
    expect(lastSource().startedAt).toBe(1);
    expect(FakeContext.made).toHaveLength(1);
  });

  it("keeps a speaker awake after its last sound, and then lets it sleep", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const { play, KEEP_AWAKE_MS, WAKE_SECONDS } = await speakers();
    (await play(decode, null)).stop();
    await vi.advanceTimersByTimeAsync(KEEP_AWAKE_MS - 1);
    expect(lastContext().state).toBe("running");
    const looping = await play(decode, null, true);
    expect(lastSource().loop).toBe(true);
    await vi.advanceTimersByTimeAsync(KEEP_AWAKE_MS * 2);
    expect(lastContext().state).toBe("running");
    looping.stop();
    await looping.ended;
    await vi.advanceTimersByTimeAsync(KEEP_AWAKE_MS);
    expect(lastContext().state).toBe("closed");
    await play(decode, null);
    expect(FakeContext.made).toHaveLength(2);
    expect(lastSource().startedAt).toBe(WAKE_SECONDS);
  });

  it("gives up on a speaker the browser does not start", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const { play, START_LIMIT_MS } = await speakers();
    FakeContext.refuse = true;
    const refused = play(decode, null);
    const settled = expect(refused).rejects.toThrow();
    await vi.advanceTimersByTimeAsync(START_LIMIT_MS);
    await settled;
    expect(lastContext().state).toBe("closed");
    FakeContext.refuse = false;
    await play(decode, null);
    expect(FakeContext.made).toHaveLength(2);
  });

  it("wakes a speaker the system suspended as it wakes a new one", async () => {
    const { play, WAKE_SECONDS } = await speakers();
    await play(decode, null);
    const context = lastContext();
    context.currentTime = 5;
    context.state = "suspended";
    await play(decode, null);
    expect(lastSource().startedAt).toBe(5 + WAKE_SECONDS);
    expect(context.state).toBe("running");
  });

  it("sends the context itself to the chosen device where it can", async () => {
    vi.stubGlobal("AudioContext", ChoosingContext);
    const { play } = await speakers();
    await play(decode, desk);
    expect(lastContext().options?.sinkId).toBe(desk.id);
    expect(lastSource().input).toBe(lastContext().destination);
    expect(FakeElement.made).toHaveLength(0);
  });

  it("plays the context's stream through an element where only an element can choose", async () => {
    Object.defineProperty(HTMLMediaElement.prototype, "setSinkId", {
      value: () => Promise.resolve(),
      configurable: true,
    });
    const { play } = await speakers();
    await play(decode, desk);
    const [element] = FakeElement.made;
    const [stream] = lastContext().streams;
    expect(element?.sinkId).toBe(desk.id);
    expect(element?.playing).toBe(true);
    expect(element?.srcObject).toBe(stream?.stream);
    expect(lastSource().input).toBe(stream);
    expect(lastContext().options?.sinkId).toBeUndefined();
  });

  it("plays through the default where nothing can choose the device", async () => {
    const { play } = await speakers();
    await play(decode, desk);
    expect(lastContext().options?.sinkId).toBeUndefined();
    expect(lastSource().input).toBe(lastContext().destination);
    expect(FakeElement.made).toHaveLength(0);
  });

  it("keeps a speaker for each device", async () => {
    vi.stubGlobal("AudioContext", ChoosingContext);
    const { play } = await speakers();
    await play(decode, desk);
    await play(decode, null);
    await play(decode, desk);
    expect(FakeContext.made.map((context) => context.options?.sinkId)).toEqual([
      desk.id,
      undefined,
    ]);
  });
});
