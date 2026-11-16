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

export interface CallReport {
  readonly status: VoiceCallState["status"];
  readonly error: string | null;
  readonly connection: string | null;
  readonly cameras: readonly string[];
  readonly screens: readonly string[];
  readonly sharing: boolean;
  readonly streams: readonly StreamReport[];
}

declare global {
  interface Window {
    aspenCall?: {
      join(session: Session, channel: string): Promise<void>;
      startCamera(): Promise<void>;
      startScreenShare(): Promise<void>;
      showVideo(): void;
      setMuted(muted: boolean): void;
      report(): Promise<CallReport>;
      loudness(): Promise<number>;
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
  async report() {
    const state = call?.state;
    return {
      status: state?.status ?? "idle",
      error: state?.error ?? null,
      connection: connection?.connectionState ?? null,
      cameras: state?.cameras.map((camera) => camera.user) ?? [],
      screens: state?.screens.map((screen) => screen.user) ?? [],
      sharing: state?.sharingScreen ?? false,
      streams: await streams(),
    };
  },
  leave() {
    call?.leave();
  },
};
