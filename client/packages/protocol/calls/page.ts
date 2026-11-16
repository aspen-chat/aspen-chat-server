/**
 * The page `scripts/check_calls.py` opens in real browsers: a voice call made by this package's
 * own `VoiceCall`, `AspenClient`, and `browserVoiceMedia`, as the app makes one, driven through
 * `window.aspenCall` by `drive.ts` and reporting what the browser's peer connection measured.
 * The deployment under test serves it as its web client, so it is at the API server's origin.
 */

import {
  AspenClient,
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
  readonly streams: readonly StreamReport[];
}

/**
 * How far apart a stream's sound and picture play, in milliseconds of the sender's clock, while
 * its picture is held back by `delayMs`: the picture's playout estimate less the sound's, `null`
 * while the browser has no estimate for both, which it makes only for tracks it synchronises.
 */
export interface SyncReport {
  readonly stream: string;
  readonly audioMid: string | null;
  readonly videoMid: string | null;
  readonly offsets: readonly (number | null)[];
  /** Each side's SSRC as last measured, and how many sender reports the browser had for it. */
  readonly sides: Readonly<Record<"audio" | "video", { ssrc: number; reports: number } | null>>;
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
      report(): Promise<CallReport>;
      loudness(): Promise<number>;
      sync(
        stream: string,
        delayMs: number,
        toleranceMs: number,
        seconds: number,
      ): Promise<SyncReport>;
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

function number(value: unknown): number {
  return typeof value === "number" ? value : 0;
}

const NO_SIDES: SyncReport["sides"] = { audio: null, video: null };

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
      codec: codecs.get(String(entry.codecId)) ?? "",
    });
  });
  return reports;
}

window.aspenCall = {
  async join(session, channel) {
    const sessionStore = new MemorySessionStore();
    sessionStore.save(session);
    const client = new AspenClient({ baseUrl: location.origin, sessionStore });
    call = new VoiceCall({ client, media });
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
   * Holds the picture of `stream` back by `delayMs` and reads, every half second for up to
   * `seconds`, how far its sound and picture play apart, until they are within `toleranceMs`.
   * A browser that synchronises them delays the sound to match; one that does not leaves it
   * `delayMs` ahead, and reports no estimate.
   */
  async sync(stream, delayMs, toleranceMs, seconds) {
    if (!(connection instanceof RTCPeerConnection)) {
      return { stream, audioMid: null, videoMid: null, offsets: [], sides: NO_SIDES };
    }
    const pc = connection;
    const of = (kind: string) =>
      remoteSections().find(
        (s) => s.kind === kind && s.stream === stream && s.direction !== "inactive",
      )?.mid ?? null;
    const audioMid = of("audio");
    const videoMid = of("video");
    const video = pc.getTransceivers().find((t) => t.mid === videoMid)?.receiver;
    if (audioMid === null || video === undefined) {
      return { stream, audioMid, videoMid, offsets: [], sides: NO_SIDES };
    }
    video.jitterBufferTarget = delayMs;
    const offsets: (number | null)[] = [];
    let sides: SyncReport["sides"] = NO_SIDES;
    try {
      for (let i = 0; i < seconds * 2; i += 1) {
        await new Promise((resolve) => setTimeout(resolve, 500));
        const playout = new Map<string, number>();
        const ssrcs = new Map<string, number>();
        const reports = new Map<number, number>();
        (await pc.getStats()).forEach((entry: Record<string, unknown>) => {
          if (entry.type === "inbound-rtp") {
            ssrcs.set(String(entry.mid), number(entry.ssrc));
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
        sides = { audio: side(audioMid), video: side(videoMid ?? "") };
        const audio = playout.get(audioMid);
        const picture = playout.get(videoMid ?? "");
        const offset = audio === undefined || picture === undefined ? null : picture - audio;
        offsets.push(offset);
        if (offset !== null && Math.abs(offset) <= toleranceMs) {
          break;
        }
      }
    } finally {
      video.jitterBufferTarget = null;
    }
    return { stream, audioMid, videoMid, offsets, sides };
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
