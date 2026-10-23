import { type DeviceChoice, resolveDevice } from "@aspen/protocol";

/**
 * Seconds of silence a speaker plays after it wakes, before its first sound. An output device
 * that sleeps while idle (PipeWire suspends one after five seconds, a sound card's codec powers
 * down, a wireless headset stops sending) drops what it is given while it wakes, and a sound
 * started at once would lose its first moments. Where the context plays to the device itself,
 * this is counted in output the device has taken, so it waits however long the device takes to
 * start and then this much more for one that starts taking audio before it is heard; where an
 * element plays the context's stream (`open`), the context's clock does not follow the device,
 * and this is all the wait there is.
 */
export const WAKE_SECONDS = 0.15;

/**
 * How long a speaker stays awake, playing silence, after its last sound ends: sounds in that
 * time start at once, and after it the device may sleep. It is under the half minute of silence
 * after which Chromium stops feeding the device itself.
 */
export const KEEP_AWAKE_MS = 10_000;

/**
 * How long a speaker may take to start before its sound is given up. A browser starts none
 * until the page has been interacted with, and a sound held until then would be stale.
 */
export const START_LIMIT_MS = 2_000;

/** Chromium's choice of device for a context, which TypeScript's DOM types do not name. */
interface SinkOptions extends AudioContextOptions {
  sinkId?: string;
}

/**
 * One device's output: a context that stays running while sounds play and for `KEEP_AWAKE_MS`
 * after, so the device stays awake for each sound after the first.
 */
interface Speaker {
  context: AudioContext;
  /** What sounds connect to: the context's own destination, or the stream `element` plays. */
  input: AudioNode;
  /** The element playing the context's output where only an element can choose the device. */
  element: HTMLAudioElement | null;
  /** The context time from which the device is awake; no sound starts before it. */
  awakeAt: number;
  playing: number;
  sleep: ReturnType<typeof setTimeout> | null;
}

/** The open speakers by the id of their device, `""` for the system's default. */
const speakers = new Map<string, Promise<Speaker>>();

/**
 * Whether the context can be sent to a chosen device itself (Chromium), and failing that whether
 * an element can (Firefox); Safari can do neither, and plays every sound through the default.
 */
function contextChoosesDevice(): boolean {
  return "setSinkId" in AudioContext.prototype;
}

function elementChoosesDevice(): boolean {
  return typeof HTMLMediaElement !== "undefined" && "setSinkId" in HTMLMediaElement.prototype;
}

async function deviceIdOf(device: DeviceChoice | null): Promise<string> {
  if (device === null || (!contextChoosesDevice() && !elementChoosesDevice())) {
    return "";
  }
  const outputs = (await navigator.mediaDevices.enumerateDevices()).filter(
    (d) => d.kind === "audiooutput",
  );
  return resolveDevice(device, outputs) ?? "";
}

/** `started`, or a rejection once `START_LIMIT_MS` passes without it. */
function withinStartLimit(started: Promise<void>): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const limit = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      reject(new DOMException("The speaker did not start", "NotAllowedError"));
    }, START_LIMIT_MS);
  });
  return Promise.race([started, limit]).finally(() => {
    clearTimeout(timer);
  });
}

/** A context playing to the device `deviceId`, or to the default where it cannot be chosen. */
function contextFor(deviceId: string): AudioContext {
  if (deviceId !== "" && contextChoosesDevice()) {
    const options: SinkOptions = { sinkId: deviceId };
    try {
      return new AudioContext(options);
    } catch {
      // The device is gone since it was listed; the default plays instead.
    }
  }
  return new AudioContext();
}

async function open(deviceId: string): Promise<Speaker> {
  const context = contextFor(deviceId);
  let input: AudioNode = context.destination;
  let element: HTMLAudioElement | null = null;
  try {
    if (deviceId !== "" && !contextChoosesDevice()) {
      const stream = context.createMediaStreamDestination();
      element = new Audio();
      element.srcObject = stream.stream;
      input = stream;
      await element.setSinkId(deviceId).catch(() => undefined);
    }
    await withinStartLimit(context.resume());
    if (element !== null) {
      await withinStartLimit(element.play());
    }
  } catch (error) {
    element?.pause();
    void context.close().catch(() => undefined);
    throw error;
  }
  return {
    context,
    input,
    element,
    awakeAt: context.currentTime + WAKE_SECONDS,
    playing: 0,
    sleep: null,
  };
}

/** Closes `speaker` once it has played nothing for `KEEP_AWAKE_MS`. */
function sleepWhenIdle(deviceId: string, speaker: Speaker): void {
  if (speaker.playing > 0 || speaker.sleep !== null) {
    return;
  }
  speaker.sleep = setTimeout(() => {
    speaker.sleep = null;
    if (speaker.playing > 0) {
      return;
    }
    speakers.delete(deviceId);
    speaker.element?.pause();
    void speaker.context.close().catch(() => undefined);
  }, KEEP_AWAKE_MS);
}

/** The speaker for `deviceId`, awake, with one more sound counted as playing on it. */
async function wake(deviceId: string): Promise<Speaker> {
  let opening = speakers.get(deviceId);
  if (opening === undefined) {
    opening = open(deviceId);
    speakers.set(deviceId, opening);
    opening.catch(() => {
      if (speakers.get(deviceId) === opening) {
        speakers.delete(deviceId);
      }
    });
  }
  const speaker = await opening;
  if (speaker.context.state === "closed") {
    // The browser closed it under us (the device went away); open it afresh.
    if (speakers.get(deviceId) === opening) {
      speakers.delete(deviceId);
    }
    return wake(deviceId);
  }
  if (speaker.sleep !== null) {
    clearTimeout(speaker.sleep);
    speaker.sleep = null;
  }
  speaker.playing += 1;
  if (speaker.context.state !== "running") {
    // Suspended by the system (Safari's "interrupted" during a phone call, say): the device
    // sleeps again, and wakes as a new one does.
    try {
      await withinStartLimit(speaker.context.resume());
    } catch (error) {
      speaker.playing -= 1;
      sleepWhenIdle(deviceId, speaker);
      throw error;
    }
    speaker.awakeAt = Math.max(speaker.awakeAt, speaker.context.currentTime + WAKE_SECONDS);
  }
  return speaker;
}

/** A sound started by `play`. */
export interface Playing {
  /** Stops it; it then counts as ended. */
  stop(): void;
  /** Settles when it has ended or been stopped. */
  ended: Promise<void>;
}

/**
 * Plays the recording `decode` gives through `device`, or the system's default where the device
 * cannot be chosen or is gone, starting once the device is awake. A looped one plays until
 * stopped. Rejects when the browser does not start the speaker (`START_LIMIT_MS`) or the
 * recording cannot be decoded.
 */
export async function play(
  decode: (context: BaseAudioContext) => Promise<AudioBuffer>,
  device: DeviceChoice | null,
  loop = false,
): Promise<Playing> {
  const deviceId = await deviceIdOf(device);
  const speaker = await wake(deviceId);
  let source: AudioBufferSourceNode;
  try {
    source = speaker.context.createBufferSource();
    source.buffer = await decode(speaker.context);
  } catch (error) {
    speaker.playing -= 1;
    sleepWhenIdle(deviceId, speaker);
    throw error;
  }
  source.loop = loop;
  source.connect(speaker.input);
  const ended = new Promise<void>((resolve) => {
    source.addEventListener(
      "ended",
      () => {
        source.disconnect();
        speaker.playing -= 1;
        sleepWhenIdle(deviceId, speaker);
        resolve();
      },
      { once: true },
    );
  });
  source.start(Math.max(speaker.context.currentTime, speaker.awakeAt));
  let stopped = false;
  return {
    stop: () => {
      if (!stopped) {
        stopped = true;
        source.stop();
      }
    },
    ended,
  };
}
