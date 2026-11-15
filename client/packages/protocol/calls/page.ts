/**
 * The page `scripts/check_calls.py` opens in real browsers: a voice call made by this package's
 * own `VoiceCall`, `AspenClient`, and `browserVoiceMedia`, as the app makes one, driven through
 * `window.aspenCall` by `drive.ts` and reporting what the browser's peer connection measured.
 * The deployment under test serves it as its web client, so it is at the API server's origin.
 */

import {
  AspenClient,
  DEFAULT_DEVICE,
  MemorySessionStore,
  VoiceCall,
  allowLoopbackPlainText,
  browserVoiceMedia,
  type CallConnection,
  type Session,
  type VoiceCallState,
} from "../src";

/** What each received or sent stream of the call measured, by its media section. */
export interface StreamReport {
  readonly mid: string | null;
  readonly kind: string;
  readonly direction: "inbound" | "outbound";
  readonly packets: number;
  readonly bytes: number;
  readonly lost: number;
  readonly framesDecoded: number;
  readonly samples: number;
  readonly concealed: number;
  /** Received audio's `totalAudioEnergy`: how loud what arrived has been, summed over time. */
  readonly energy: number;
  /** Received audio's `audioLevel`, the moment's loudness, 0 to 1. */
  readonly level: number;
  /** The codec as negotiated: its MIME type and format parameters. */
  readonly codec: string;
}

/** A media section of the server's description. */
export interface SectionReport {
  readonly mid: string | null;
  readonly kind: string;
  readonly direction: "sendrecv" | "sendonly" | "recvonly" | "inactive";
  readonly stream: string | null;
  readonly ssrcs: readonly number[];
}

export interface CallReport {
  readonly status: VoiceCallState["status"];
  readonly error: string | null;
  readonly connection: string | null;
  readonly cameras: readonly string[];
  readonly screens: readonly string[];
  readonly sharing: boolean;
  /** The media sections the server's description has, idle ones among them. */
  readonly sections: readonly SectionReport[];
  /** The media sections of the browser's own answer, as it last applied one. */
  readonly answered: readonly SectionReport[];
  /** The extensions the latest signalling socket negotiated (`WebSocket.extensions`). */
  readonly signalling: string | null;
  readonly streams: readonly StreamReport[];
}

/**
 * How far apart a stream's sound and picture play, in milliseconds of the sender's clock, while
 * its picture is held back by `delayMs`: the picture's playout estimate less the sound's, read
 * every half second, `null` while the browser has no estimate for both, which it makes only for
 * tracks it synchronises.
 */
export interface SyncReport {
  readonly stream: string;
  readonly audioMid: string | null;
  readonly videoMid: string | null;
  readonly offsets: readonly (number | null)[];
  /** Each side's SSRC as last measured, and how many sender reports the browser had for it. */
  readonly sides: Readonly<Record<"audio" | "video", { ssrc: number; reports: number } | null>>;
  /** Where each side's delay went over the last half second measured. */
  readonly delays: Readonly<Record<"audio" | "video", DelayReport | null>>;
}

/**
 * A received track's delays in milliseconds, averaged over an interval: what its jitter buffer
 * held each sample or frame, what it aimed to hold (the synchronised target among what it
 * weighs), and for sound what the output device added after it (`media-playout`, where the
 * browser reports one). `null` where the browser emitted nothing to average over.
 */
export interface DelayReport {
  readonly jitterBuffer: number | null;
  readonly target: number | null;
  readonly playout: number | null;
}

declare global {
  interface Window {
    aspenCall?: {
      join(session: Session, channel: string): Promise<void>;
      startCamera(): Promise<void>;
      startScreenShare(): Promise<void>;
      stopCamera(): void;
      stopScreenShare(): void;
      showVideo(): void;
      setMuted(muted: boolean): void;
      soundOutput(): Promise<AudioContextState | "timedOut">;
      report(): Promise<CallReport>;
      loudness(): Promise<number>;
      sync(streams: readonly string[], delayMs: number, seconds: number): Promise<SyncReport[]>;
      reference(delayMs: number, seconds: number): Promise<SyncReport>;
      leave(): void;
    };
  }
}

allowLoopbackPlainText(true);
const media = browserVoiceMedia();
let connection: CallConnection | null = null;
const createConnection = media.createConnection.bind(media);
media.createConnection = async () => {
  connection = await createConnection();
  return connection;
};
let call: VoiceCall | null = null;

/** The call's signalling sockets, the latest last, so a report can say what each negotiated. */
const sockets: WebSocket[] = [];
class RecordedSocket extends WebSocket {
  constructor(url: string | URL, protocols?: string | string[]) {
    super(url, protocols);
    sockets.push(this);
  }
}

function number(value: unknown): number {
  return typeof value === "number" ? value : 0;
}

const NO_SIDES: SyncReport["sides"] = { audio: null, video: null };
const NO_DELAYS: SyncReport["delays"] = { audio: null, video: null };

/** The cumulative counters a `DelayReport` is averaged from, as one `getStats` read them. */
interface DelayCounters {
  readonly jitterBuffer: number;
  readonly target: number;
  readonly emitted: number;
  readonly playout: number;
  readonly played: number;
}

/** Milliseconds per item of what `total` (in seconds) rose by while `count` rose, if it did. */
function average(total: number, count: number): number | null {
  return count > 0 ? Math.round((total / count) * 1000) : null;
}

function delaysBetween(before: DelayCounters | undefined, after: DelayCounters | undefined) {
  if (before === undefined || after === undefined) {
    return null;
  }
  const emitted = after.emitted - before.emitted;
  return {
    jitterBuffer: average(after.jitterBuffer - before.jitterBuffer, emitted),
    target: average(after.target - before.target, emitted),
    playout: average(after.playout - before.playout, after.played - before.played),
  };
}

/**
 * The media sections of the remote description, as the server described them: SDP's `m=` lines
 * begin sections, whose `a=mid`, direction, `a=msid`, and `a=ssrc` attributes say which each is,
 * whether it carries anything, the stream it belongs to, and the SSRCs it is sent on.
 */
function remoteSections(): SectionReport[] {
  return sectionsOf(connection instanceof RTCPeerConnection ? connection.remoteDescription : null);
}

/** The media sections of a description, as `remoteSections` reads them. */
function sectionsOf(description: RTCSessionDescription | null): SectionReport[] {
  const sdp = description?.sdp ?? "";
  return sdp
    .split(/\r?\nm=/)
    .slice(1)
    .map((section) => {
      const lines = section.split(/\r?\n/);
      const attribute = (name: string) =>
        lines.find((line) => line.startsWith(`a=${name}:`))?.slice(name.length + 3);
      const ssrcs = new Set(
        lines
          .filter((line) => line.startsWith("a=ssrc:"))
          .map((line) => Number(line.slice("a=ssrc:".length).split(" ")[0])),
      );
      return {
        mid: attribute("mid") ?? null,
        kind: lines[0]?.split(" ")[0] ?? "",
        direction: DIRECTIONS.find((direction) => lines.includes(`a=${direction}`)) ?? "sendrecv",
        stream: attribute("msid")?.split(" ")[0] ?? null,
        ssrcs: [...ssrcs],
      };
    });
}

const DIRECTIONS = ["sendrecv", "sendonly", "recvonly", "inactive"] as const;

async function streams(): Promise<StreamReport[]> {
  if (!(connection instanceof RTCPeerConnection)) {
    return [];
  }
  const stats = await connection.getStats();
  const codecs = new Map<string, string>();
  stats.forEach((entry: Record<string, unknown>) => {
    if (entry.type === "codec") {
      const fmtp = typeof entry.sdpFmtpLine === "string" ? entry.sdpFmtpLine : "";
      codecs.set(String(entry.id), `${String(entry.mimeType)} ${fmtp}`);
    }
  });
  const reports: StreamReport[] = [];
  stats.forEach((entry: Record<string, unknown>) => {
    if (entry.type !== "inbound-rtp" && entry.type !== "outbound-rtp") {
      return;
    }
    const inbound = entry.type === "inbound-rtp";
    reports.push({
      mid: typeof entry.mid === "string" ? entry.mid : null,
      kind: String(entry.kind),
      direction: inbound ? "inbound" : "outbound",
      packets: number(inbound ? entry.packetsReceived : entry.packetsSent),
      bytes: number(inbound ? entry.bytesReceived : entry.bytesSent),
      lost: number(entry.packetsLost),
      framesDecoded: number(entry.framesDecoded),
      samples: number(entry.totalSamplesReceived),
      concealed: number(entry.concealedSamples),
      energy: number(entry.totalAudioEnergy),
      level: number(entry.audioLevel),
      codec: codecs.get(String(entry.codecId)) ?? "",
    });
  });
  return reports;
}

/** A stream to measure: its sound's and its picture's sections, and the picture's receiver. */
interface Pair {
  readonly stream: string;
  readonly audioMid: string;
  readonly videoMid: string;
  readonly video: RTCRtpReceiver;
}

/**
 * Holds the picture of each pair back by `delayMs` and reads, every half second for `seconds`,
 * how far its sound and picture play apart. A browser that synchronises them delays the sound
 * to match, closing the gap over tens of seconds; one that does not leaves it `delayMs` ahead,
 * and reports no estimate.
 */
async function measure(
  pc: RTCPeerConnection,
  pairs: readonly Pair[],
  delayMs: number,
  seconds: number,
): Promise<SyncReport[]> {
  const measured = pairs.map((pair) => ({
    pair,
    offsets: [] as (number | null)[],
    sides: NO_SIDES,
    delays: NO_DELAYS,
  }));
  for (const { video } of pairs) {
    video.jitterBufferTarget = delayMs;
  }
  let counted = new Map<string, DelayCounters>();
  try {
    for (let i = 0; i < seconds * 2; i += 1) {
      await new Promise((resolve) => setTimeout(resolve, 500));
      const playout = new Map<string, number>();
      const ssrcs = new Map<string, number>();
      const reports = new Map<number, number>();
      const stats = await pc.getStats();
      const counters = new Map<string, DelayCounters>();
      stats.forEach((entry: Record<string, unknown>) => {
        if (entry.type === "inbound-rtp") {
          ssrcs.set(String(entry.mid), number(entry.ssrc));
          const device: Record<string, unknown> | undefined =
            typeof entry.playoutId === "string"
              ? (stats.get(entry.playoutId) as Record<string, unknown> | undefined)
              : undefined;
          counters.set(String(entry.mid), {
            jitterBuffer: number(entry.jitterBufferDelay),
            target: number(entry.jitterBufferTargetDelay),
            emitted: number(entry.jitterBufferEmittedCount),
            playout: number(device?.totalPlayoutDelay),
            played: number(device?.totalSamplesCount),
          });
          if (typeof entry.estimatedPlayoutTimestamp === "number") {
            playout.set(String(entry.mid), entry.estimatedPlayoutTimestamp);
          }
        } else if (entry.type === "remote-outbound-rtp") {
          reports.set(number(entry.ssrc), number(entry.reportsSent));
        }
      });
      const side = (mid: string) => {
        const ssrc = ssrcs.get(mid);
        return ssrc === undefined ? null : { ssrc, reports: reports.get(ssrc) ?? 0 };
      };
      for (const m of measured) {
        const { audioMid, videoMid } = m.pair;
        m.sides = { audio: side(audioMid), video: side(videoMid) };
        m.delays = {
          audio: delaysBetween(counted.get(audioMid), counters.get(audioMid)),
          video: delaysBetween(counted.get(videoMid), counters.get(videoMid)),
        };
        const audio = playout.get(audioMid);
        const picture = playout.get(videoMid);
        m.offsets.push(audio === undefined || picture === undefined ? null : picture - audio);
      }
      counted = counters;
    }
  } finally {
    for (const { video } of pairs) {
      video.jitterBufferTarget = null;
    }
  }
  return measured.map(({ pair, offsets, sides, delays }) => ({
    stream: pair.stream,
    audioMid: pair.audioMid,
    videoMid: pair.videoMid,
    offsets,
    sides,
    delays,
  }));
}

window.aspenCall = {
  async join(session, channel) {
    const sessionStore = new MemorySessionStore();
    sessionStore.save(session);
    const client = new AspenClient({ baseUrl: location.origin, sessionStore });
    call = new VoiceCall({ client, media, WebSocket: RecordedSocket });
    await call.join(channel);
  },
  async startCamera() {
    await call?.startCamera();
  },
  async startScreenShare() {
    await call?.startScreenShare();
  },
  stopCamera() {
    call?.stopCamera();
  },
  stopScreenShare() {
    call?.stopScreenShare();
  },
  showVideo() {
    call?.showVideo();
  },
  setMuted(muted) {
    call?.setMuted(muted);
  },
  /**
   * Whether this browser can play sound at all: the state an `AudioContext` reaches, `running`
   * where it can. Firefox opens a sound device for every audio graph, its fake microphone's
   * among them, and with no sound server to open one on, its contexts stay `suspended` (and
   * `resume` never settles) and its fake microphone records silence. Opening one on PulseAudio
   * takes Firefox up to about two seconds.
   */
  async soundOutput() {
    const context = new AudioContext();
    try {
      return await Promise.race([
        context.resume().then(() => context.state),
        new Promise<"timedOut">((resolve) => {
          setTimeout(() => {
            resolve("timedOut");
          }, 10_000);
        }),
      ]);
    } finally {
      void context.close();
    }
  },
  /**
   * The loudest sample the audio slots carried over a second, read off their tracks. The
   * browser's own `audioLevel` and `totalAudioEnergy` are measured where it plays a track, and
   * Chromium reads them after the muted element the call keeps each track playing in, so they
   * are zero however loud the sound.
   */
  async loudness() {
    if (!(connection instanceof RTCPeerConnection) || call === null) {
      return 0;
    }
    const context = new AudioContext();
    const analysers = connection
      .getTransceivers()
      .filter((t) => t.receiver.track.kind === "audio" && t.currentDirection !== "sendonly")
      .map((t) => {
        const analyser = context.createAnalyser();
        context.createMediaStreamSource(new MediaStream([t.receiver.track])).connect(analyser);
        return analyser;
      });
    const samples = new Float32Array(2048);
    let peak = 0;
    for (let i = 0; i < 20; i += 1) {
      await new Promise((resolve) => setTimeout(resolve, 50));
      for (const analyser of analysers) {
        analyser.getFloatTimeDomainData(samples);
        for (const sample of samples) {
          peak = Math.max(peak, Math.abs(sample));
        }
      }
    }
    await context.close();
    return peak;
  },
  /**
   * Measures each of `streams` as `measure` does, at once, on the call's connection. A stream
   * with no sound, or no picture, is reported with no sections and no offsets.
   */
  async sync(names, delayMs, seconds) {
    const empty = (stream: string, audioMid: string | null, videoMid: string | null) => ({
      stream,
      audioMid,
      videoMid,
      offsets: [],
      sides: NO_SIDES,
      delays: NO_DELAYS,
    });
    if (!(connection instanceof RTCPeerConnection)) {
      return names.map((stream) => empty(stream, null, null));
    }
    const pc = connection;
    const of = (stream: string, kind: string) =>
      remoteSections().find(
        (s) => s.kind === kind && s.stream === stream && s.direction !== "inactive",
      )?.mid ?? null;
    const found = names.map((stream) => {
      const audioMid = of(stream, "audio");
      const videoMid = of(stream, "video");
      const video = pc.getTransceivers().find((t) => t.mid === videoMid)?.receiver;
      return { stream, audioMid, videoMid, video };
    });
    const pairs = found.flatMap(({ stream, audioMid, videoMid, video }) =>
      audioMid === null || videoMid === null || video === undefined
        ? []
        : [{ stream, audioMid, videoMid, video }],
    );
    const measured = await measure(pc, pairs, delayMs, seconds);
    return found.map(
      ({ stream, audioMid, videoMid }) =>
        measured.find((m) => m.stream === stream) ?? empty(stream, audioMid, videoMid),
    );
  },
  /**
   * Measures, as `sync` does, a camera and microphone of this browser's sent straight to it over
   * a second pair of peer connections, played as the call plays sound: how far this browser
   * itself leaves the two apart under the hold, which the call's streams are judged against.
   */
  async reference(delayMs, seconds) {
    const own = browserVoiceMedia();
    const sender = (await own.createConnection()) as RTCPeerConnection;
    const receiver = (await own.createConnection()) as RTCPeerConnection;
    const tracks = [
      await own.getMicrophone(DEFAULT_DEVICE, 1),
      await own.getCamera(DEFAULT_DEVICE),
    ];
    try {
      const candidates = (to: RTCPeerConnection) => (event: RTCPeerConnectionIceEvent) => {
        if (event.candidate !== null) {
          void to.addIceCandidate(event.candidate);
        }
      };
      sender.onicecandidate = candidates(receiver);
      receiver.onicecandidate = candidates(sender);
      receiver.ontrack = (event) => {
        if (event.track.kind === "audio") {
          own.play("reference", event.track);
        }
      };
      const stream = new MediaStream(tracks);
      for (const track of tracks) {
        sender.addTrack(track, stream);
      }
      const offer = await sender.createOffer();
      await sender.setLocalDescription(offer);
      await receiver.setRemoteDescription(offer);
      const answer = await receiver.createAnswer();
      await receiver.setLocalDescription(answer);
      await sender.setRemoteDescription(answer);
      const mid = (kind: string) =>
        receiver.getTransceivers().find((t) => t.receiver.track.kind === kind);
      const audio = mid("audio");
      const video = mid("video");
      if (audio?.mid == null || video?.mid == null) {
        throw new Error("the reference connection has no sound or no picture");
      }
      const [measured] = await measure(
        receiver,
        [{ stream: "reference", audioMid: audio.mid, videoMid: video.mid, video: video.receiver }],
        delayMs,
        seconds,
      );
      if (measured === undefined) {
        throw new Error("the reference was not measured");
      }
      return measured;
    } finally {
      own.stop("reference");
      for (const track of tracks) {
        track.stop();
      }
      sender.close();
      receiver.close();
    }
  },
  async report() {
    const state = call?.state;
    return {
      status: state?.status ?? "idle",
      error: state?.error ?? null,
      connection: connection?.connectionState ?? null,
      cameras: state?.cameras.map((camera) => camera.user) ?? [],
      screens: state?.screens.map((screen) => screen.user) ?? [],
      sharing: state?.sharingScreen ?? false,
      sections: remoteSections(),
      signalling: sockets.at(-1)?.extensions ?? null,
      answered: sectionsOf(
        connection instanceof RTCPeerConnection ? connection.currentLocalDescription : null,
      ),
      streams: await streams(),
    };
  },
  leave() {
    call?.leave();
  },
};
