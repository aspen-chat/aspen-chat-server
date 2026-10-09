/**
 * Files sent between the people of a voice call, over a WebRTC data channel of their own.
 *
 * The voice server keeps the offers and introduces the two sides of each transfer
 * (`voice_protocol::signal` describes the flow); the bytes never pass through its media path.
 * Once both sides are told `transferStarting`, the sender opens a peer connection with the ICE
 * servers that frame names (its STUN, and its TURN relay with credentials for this transfer
 * alone) and a data channel, and the receiver answers. `relayOnly` gathers only relay
 * candidates, so neither side learns the other's address; `directPreferred` also gathers direct
 * ones, and a hole-punched connection wins when one can be made. The data channel is encrypted
 * end to end either way.
 *
 * The sender's side runs without its user: once they have offered a file, every acceptance
 * starts sending on its own, so an offer works while its sender is away. A transfer outlives its
 * offer; either side may cancel it at any time, which closes it at once. Nothing resumes: a
 * transfer that fails or is cancelled has to be accepted again while the offer stands.
 */

import type {
  ClientMessage,
  FileOffer,
  ServerMessage,
  TransferEnd,
  TransferLink,
  TransferMode,
} from "./generated/voiceSignal";
import { plainFileName } from "./fileNames";

export type { TransferLink, TransferMode };

/** The default time an offer stands, in seconds. */
export const DEFAULT_OFFER_SECONDS = 60;
/** The shortest and longest an offer may stand, as the voice server holds them. */
export const MIN_OFFER_SECONDS = 10;
export const MAX_OFFER_SECONDS = 60 * 60;
/** The most bytes sent in one data channel message. */
const CHUNK_BYTES = 64 * 1024;
/** Sending pauses above this many bytes waiting in the channel, and resumes below the low mark. */
const BUFFER_HIGH_BYTES = 4 * 1024 * 1024;
const BUFFER_LOW_BYTES = 1024 * 1024;
/** How often progress is reported, at most. */
const PROGRESS_INTERVAL_MS = 200;
/** What the receiver sends down the channel once every byte is in, ahead of closing it. */
const DONE = "done";
/**
 * How long a sender whose channel closed after its last byte waits for the receiver's word
 * before calling the transfer failed.
 */
const VERDICT_GRACE_MS = 5000;
/** How often a live transfer's route is looked at again, since ICE may move it. */
const ROUTE_INTERVAL_MS = 3000;

/** A file offered in the call, with when it stops standing on this device's clock. */
export interface OfferState {
  readonly id: string;
  readonly from: string;
  readonly name: string;
  readonly size: number;
  readonly allowDirect: boolean;
  /** `Date.now()` at which the offer can no longer be accepted. */
  readonly expiresAt: number;
  /** Whether this device made it. */
  readonly own: boolean;
}

export type TransferStatus = "connecting" | "moving" | TransferEnd;

/**
 * How a transfer's bytes actually travel, whatever was preferred: straight between the two
 * devices, or through the voice server's relay. `null` until the connection is up.
 */
export type TransferRoute = "direct" | "relayed";

/** A transfer this device is one side of. */
export interface TransferState {
  readonly offer: string;
  readonly peer: string;
  readonly role: "sender" | "receiver";
  readonly mode: TransferMode;
  readonly name: string;
  readonly size: number;
  /** Bytes sent, or received, so far. */
  readonly bytes: number;
  readonly status: TransferStatus;
  /** The route in use, or last used once ended; `null` before the connection came up. */
  readonly route: TransferRoute | null;
  /** Who ended it, once it has ended: this device, or the other side. */
  readonly endedBy: "self" | "peer" | null;
  /** The file, on the receiving side, once every byte has arrived, unless it went to a `FileSink`. */
  readonly file: Blob | null;
  /** Whether it is being written, or was written, straight to a file the receiver chose. */
  readonly toDisk: boolean;
}

/**
 * Where a receiver writes a file as it arrives, chosen before the transfer began: a browser's
 * `FileSystemWritableFileStream` fits. `close` keeps what was written; `abort` discards it.
 */
export interface FileSink {
  write(chunk: ArrayBuffer): Promise<void>;
  close(): Promise<void>;
  abort(): Promise<void>;
}

export interface FilesState {
  /** Offers that may still be accepted, this device's own included. */
  readonly offers: readonly OfferState[];
  /** Transfers this device is one side of, finished ones until dismissed. */
  readonly transfers: readonly TransferState[];
  /** Every transfer under way in the call, one entry each, whoever its two sides are. */
  readonly links: readonly TransferLink[];
  /** The megabits a second every relayed transfer on this voice server shares; `null` when it does not relay. */
  readonly relayMbps: number | null;
}

export const NO_FILES: FilesState = { offers: [], transfers: [], links: [], relayMbps: null };

/** What the transfers need from the browser, narrowed so they run and are tested without one. */
export type PeerConnectionFactory = (configuration: RTCConfiguration) => RTCPeerConnection;

export interface FileTransfersOptions {
  /** Sends a frame to the voice server. */
  send: (frame: ClientMessage) => void;
  onChange: () => void;
  createPeerConnection?: PeerConnectionFactory;
  now?: () => number;
  randomId?: () => string;
}

/** What a transfer's peer connection carries through the voice server. */
type Signal =
  | { kind: "description"; description: RTCSessionDescriptionInit }
  | { kind: "candidate"; candidate: RTCIceCandidateInit };

interface Live {
  state: TransferState;
  pc: RTCPeerConnection;
  channel: RTCDataChannel | null;
  /** Candidates that arrived before the remote description. */
  pending: RTCIceCandidateInit[];
  received: ArrayBuffer[];
  lastReport: number;
  routeTimer: ReturnType<typeof setInterval> | null;
  /** Where the receiver writes, and the writes still under way, in order. */
  sink: FileSink | null;
  writing: Promise<void>;
}

/**
 * The route of a peer connection's selected candidate pair: relayed when either end is a relay
 * candidate. Chromium names the pair from its transport; Firefox marks it nominated.
 */
export async function routeOf(pc: RTCPeerConnection): Promise<TransferRoute | null> {
  const stats = await pc.getStats();
  let pair: { localCandidateId?: string; remoteCandidateId?: string } | undefined;
  stats.forEach((report: { type: string; selectedCandidatePairId?: string }) => {
    if (report.type === "transport" && report.selectedCandidatePairId !== undefined) {
      pair = stats.get(report.selectedCandidatePairId) as typeof pair;
    }
  });
  if (pair === undefined) {
    stats.forEach(
      (report: {
        type: string;
        nominated?: boolean;
        state?: string;
        localCandidateId?: string;
        remoteCandidateId?: string;
      }) => {
        if (
          report.type === "candidate-pair" &&
          report.nominated === true &&
          report.state === "succeeded"
        ) {
          pair = report;
        }
      },
    );
  }
  if (pair?.localCandidateId === undefined || pair.remoteCandidateId === undefined) {
    return null;
  }
  const local = stats.get(pair.localCandidateId) as { candidateType?: string } | undefined;
  const remote = stats.get(pair.remoteCandidateId) as { candidateType?: string } | undefined;
  if (local === undefined || remote === undefined) {
    return null;
  }
  return local.candidateType === "relay" || remote.candidateType === "relay" ? "relayed" : "direct";
}

function key(offer: string, peer: string): string {
  return `${offer}:${peer}`;
}

export class FileTransfers {
  readonly #send: (frame: ClientMessage) => void;
  readonly #onChange: () => void;
  readonly #createPeerConnection: PeerConnectionFactory;
  readonly #now: () => number;
  readonly #randomId: () => string;
  #offers = new Map<string, OfferState>();
  readonly #live = new Map<string, Live>();
  /** Finished transfers, kept for the UI until dismissed. */
  readonly #finished = new Map<string, TransferState>();
  /** Sinks for offers this device accepted, until their transfers start. */
  readonly #sinks = new Map<string, FileSink>();
  /** The files this device offered, kept while the offer stands or a transfer of it goes on. */
  readonly #files = new Map<string, Blob>();
  #links: TransferLink[] = [];
  #relayMbps: number | null = null;
  #me: string | null = null;
  #snapshot: FilesState = NO_FILES;

  constructor(options: FileTransfersOptions) {
    this.#send = options.send;
    this.#onChange = options.onChange;
    this.#createPeerConnection =
      options.createPeerConnection ?? ((configuration) => new RTCPeerConnection(configuration));
    this.#now = options.now ?? (() => Date.now());
    this.#randomId = options.randomId ?? (() => crypto.randomUUID());
  }

  get state(): FilesState {
    return this.#snapshot;
  }

  /** Starts over from a `ready` frame: the call's offers, links, and relay policy. */
  reset(ready: Extract<ServerMessage, { type: "ready" }>): void {
    this.closeAll();
    this.#me = ready.user;
    this.#offers = new Map(
      (ready.offers ?? []).map((offer) => [offer.id, this.#offerState(offer)]),
    );
    this.#links = [...(ready.links ?? [])];
    this.#relayMbps = ready.transfers?.relayMbps ?? null;
    this.#changed();
  }

  /** Closes every transfer at once, as when leaving the call; the voice server ends them too. */
  closeAll(): void {
    for (const live of this.#live.values()) {
      live.pc.close();
      void live.sink?.abort().catch(() => undefined);
    }
    for (const offer of [...this.#sinks.keys()]) {
      this.#dropSink(offer);
    }
    this.#live.clear();
    this.#finished.clear();
    this.#files.clear();
    this.#offers.clear();
    this.#links = [];
    this.#changed();
  }

  /** Offers `file` to the call for `validForSeconds`, letting receivers connect directly when `allowDirect`. */
  offer(file: Blob, name: string, allowDirect: boolean, validForSeconds: number): string {
    const id = this.#randomId();
    this.#files.set(id, file);
    this.#send({
      type: "offerFile",
      offer: id,
      name,
      size: file.size,
      allowDirect,
      validForSeconds: Math.round(
        Math.min(MAX_OFFER_SECONDS, Math.max(MIN_OFFER_SECONDS, validForSeconds)),
      ),
    });
    return id;
  }

  /** Takes back one of this device's offers; transfers already started go on. */
  withdraw(offer: string): void {
    this.#send({ type: "withdrawFile", offer });
  }

  /**
   * Accepts an offer, in the mode the user acknowledged, writing what arrives to `sink` when
   * the user chose where to save it, and holding it for `file` otherwise.
   */
  accept(offer: string, mode: TransferMode, sink?: FileSink): void {
    if (sink !== undefined) {
      this.#dropSink(offer);
      this.#sinks.set(offer, sink);
    }
    this.#send({ type: "acceptFile", offer, mode });
  }

  /** Discards the file a sink was opened for, when its transfer will not start. */
  #dropSink(offer: string): void {
    const sink = this.#sinks.get(offer);
    if (sink !== undefined) {
      this.#sinks.delete(offer);
      void sink.abort().catch(() => undefined);
    }
  }

  /** Cancels one transfer at once. */
  cancel(offer: string, peer: string): void {
    const live = this.#live.get(key(offer, peer));
    if (live === undefined) {
      return;
    }
    this.#send({ type: "endTransfer", offer, peer, reason: "cancelled" });
    this.#end(live, "cancelled", "self");
  }

  /** Forgets a finished transfer, and the file it received. */
  dismiss(offer: string, peer: string): void {
    if (this.#finished.delete(key(offer, peer))) {
      this.#changed();
    }
  }

  /** Takes a frame about files; false for any other. */
  handle(frame: ServerMessage): boolean {
    switch (frame.type) {
      case "fileOffered":
        this.#offers.set(frame.offer.id, this.#offerState(frame.offer));
        this.#changed();
        return true;
      case "fileWithdrawn":
        this.#offers.delete(frame.offer);
        this.#dropSink(frame.offer);
        this.#releaseFile(frame.offer);
        this.#changed();
        return true;
      case "transferLinkChanged":
        if (frame.active) {
          this.#links = [...this.#links, frame.link];
        } else {
          const index = this.#links.findIndex(
            (link) => link.sender === frame.link.sender && link.receiver === frame.link.receiver,
          );
          if (index >= 0) {
            this.#links = this.#links.filter((_, i) => i !== index);
          }
        }
        this.#changed();
        return true;
      case "transferStarting":
        this.#start(frame);
        return true;
      case "transferSignal": {
        const live = this.#live.get(key(frame.offer, frame.peer));
        if (live !== undefined) {
          void this.#signal(live, frame.signal as Signal);
        }
        return true;
      }
      case "transferEnded": {
        const live = this.#live.get(key(frame.offer, frame.peer));
        if (live !== undefined) {
          this.#end(live, frame.reason, "peer");
        }
        return true;
      }
      default:
        return false;
    }
  }

  #offerState(offer: FileOffer): OfferState {
    return {
      id: offer.id,
      from: offer.from,
      name: plainFileName(offer.name),
      size: offer.size,
      allowDirect: offer.allowDirect,
      expiresAt: this.#now() + offer.expiresInMs,
      own: offer.from === this.#me,
    };
  }

  #start(frame: Extract<ServerMessage, { type: "transferStarting" }>): void {
    const file = this.#files.get(frame.offer);
    const sending = frame.role === "sender";
    if (sending && file === undefined) {
      // This device no longer holds the file (it left and came back): nothing to send.
      this.#send({ type: "endTransfer", offer: frame.offer, peer: frame.peer, reason: "failed" });
      return;
    }
    const pc = this.#createPeerConnection({
      iceServers: frame.iceServers.map((server) => ({
        urls: server.urls,
        ...(server.username === undefined || server.username === null
          ? {}
          : { username: server.username }),
        ...(server.credential === undefined || server.credential === null
          ? {}
          : { credential: server.credential }),
      })),
      iceTransportPolicy: frame.mode === "relayOnly" ? "relay" : "all",
    });
    const live: Live = {
      state: {
        offer: frame.offer,
        peer: frame.peer,
        role: frame.role,
        mode: frame.mode,
        name: plainFileName(frame.name),
        size: sending ? (file?.size ?? frame.size) : frame.size,
        bytes: 0,
        status: "connecting",
        route: null,
        endedBy: null,
        file: null,
        toDisk: false,
      },
      pc,
      channel: null,
      pending: [],
      received: [],
      lastReport: 0,
      routeTimer: null,
      sink: null,
      writing: Promise.resolve(),
    };
    if (!sending) {
      live.sink = this.#sinks.get(frame.offer) ?? null;
      this.#sinks.delete(frame.offer);
      live.state = { ...live.state, toDisk: live.sink !== null };
    }
    this.#live.set(key(frame.offer, frame.peer), live);
    pc.onicecandidate = (event) => {
      if (event.candidate !== null) {
        this.#sendSignal(live, { kind: "candidate", candidate: event.candidate.toJSON() });
      }
    };
    pc.onconnectionstatechange = () => {
      if (pc.connectionState === "failed") {
        this.#fail(live);
      } else if (pc.connectionState === "connected" && live.routeTimer === null) {
        void this.#checkRoute(live);
        live.routeTimer = setInterval(() => {
          void this.#checkRoute(live);
        }, ROUTE_INTERVAL_MS);
      }
    };
    if (sending && file !== undefined) {
      const channel = pc.createDataChannel("file", { ordered: true });
      this.#sendOver(live, channel, file);
      void (async () => {
        try {
          await pc.setLocalDescription(await pc.createOffer());
          if (pc.localDescription !== null) {
            this.#sendSignal(live, {
              kind: "description",
              description: pc.localDescription.toJSON(),
            });
          }
        } catch {
          this.#fail(live);
        }
      })();
    } else {
      pc.ondatachannel = (event) => {
        this.#receiveOver(live, event.channel);
      };
    }
    this.#changed();
  }

  async #signal(live: Live, signal: Signal): Promise<void> {
    const pc = live.pc;
    try {
      if (signal.kind === "description") {
        await pc.setRemoteDescription(signal.description);
        for (const candidate of live.pending.splice(0)) {
          await pc.addIceCandidate(candidate);
        }
        if (signal.description.type === "offer") {
          await pc.setLocalDescription(await pc.createAnswer());
          if (pc.localDescription !== null) {
            this.#sendSignal(live, {
              kind: "description",
              description: pc.localDescription.toJSON(),
            });
          }
        }
      } else if (pc.remoteDescription === null) {
        live.pending.push(signal.candidate);
      } else {
        await pc.addIceCandidate(signal.candidate);
      }
    } catch {
      this.#fail(live);
    }
  }

  #sendSignal(live: Live, signal: Signal): void {
    this.#send({
      type: "transferSignal",
      offer: live.state.offer,
      peer: live.state.peer,
      signal,
    });
  }

  /** Sends `file` once the channel opens, keeping the channel's buffer between the marks. */
  #sendOver(live: Live, channel: RTCDataChannel, file: Blob): void {
    live.channel = channel;
    channel.binaryType = "arraybuffer";
    channel.bufferedAmountLowThreshold = BUFFER_LOW_BYTES;
    channel.onopen = () => {
      this.#update(live, { status: "moving" }, true);
      const chunk = Math.min(CHUNK_BYTES, live.pc.sctp?.maxMessageSize ?? CHUNK_BYTES);
      let position = 0;
      const pump = async (): Promise<void> => {
        while (position < file.size) {
          if (!this.#isLive(live)) {
            return;
          }
          if (channel.bufferedAmount > BUFFER_HIGH_BYTES) {
            channel.onbufferedamountlow = () => {
              channel.onbufferedamountlow = null;
              void pump();
            };
            return;
          }
          const piece = await file.slice(position, position + chunk).arrayBuffer();
          if (!this.#isLive(live) || channel.readyState !== "open") {
            return;
          }
          channel.send(piece);
          position += piece.byteLength;
          this.#update(live, { bytes: position });
        }
        // Every byte is on its way; the receiver says when it has them all.
      };
      void pump().catch(() => {
        this.#fail(live);
      });
    };
    // The receiver's word that every byte arrived, sent down the channel ahead of its closing
    // so that it always comes first; its `endTransfer` tells the server.
    channel.onmessage = (event: MessageEvent) => {
      if (event.data === DONE) {
        this.#end(live, "completed", "peer");
      }
    };
    channel.onclose = () => {
      if (!this.#isLive(live)) {
        return;
      }
      if (live.state.bytes < file.size) {
        this.#fail(live);
        return;
      }
      // Everything was sent: the verdict may still be on its way through the voice server.
      setTimeout(() => {
        this.#fail(live);
      }, VERDICT_GRACE_MS);
    };
  }

  /**
   * Takes what arrives until every byte of the offered size is here: into the chosen file as it
   * comes, or held for `file`. It is done only once the file is closed on disk.
   */
  #receiveOver(live: Live, channel: RTCDataChannel): void {
    live.channel = channel;
    channel.binaryType = "arraybuffer";
    const complete = async (): Promise<void> => {
      let file: Blob | null = null;
      try {
        if (live.sink === null) {
          file = new Blob(live.received);
          live.received = [];
        } else {
          await live.writing;
          await live.sink.close();
        }
      } catch {
        this.#fail(live);
        return;
      }
      if (!this.#isLive(live)) {
        return;
      }
      // Closing a channel sends what it holds first, so this reaches the sender before the close.
      if (channel.readyState === "open") {
        channel.send(DONE);
      }
      this.#send({
        type: "endTransfer",
        offer: live.state.offer,
        peer: live.state.peer,
        reason: "completed",
      });
      this.#end(live, "completed", "self", file);
    };
    channel.onopen = () => {
      this.#update(live, { status: "moving" }, true);
      if (live.state.size === 0) {
        void complete();
      }
    };
    channel.onmessage = (event: MessageEvent) => {
      if (!(event.data instanceof ArrayBuffer)) {
        return;
      }
      const chunk = event.data;
      const bytes = live.state.bytes + chunk.byteLength;
      if (bytes > live.state.size) {
        this.#fail(live);
        return;
      }
      const sink = live.sink;
      if (sink === null) {
        live.received.push(chunk);
      } else {
        live.writing = live.writing.then(() => sink.write(chunk));
        live.writing.catch(() => {
          this.#fail(live);
        });
      }
      this.#update(live, { bytes });
      if (bytes === live.state.size) {
        void complete();
      }
    };
    channel.onclose = () => {
      if (this.#isLive(live)) {
        this.#fail(live);
      }
    };
  }

  async #checkRoute(live: Live): Promise<void> {
    try {
      const route = await routeOf(live.pc);
      if (route !== null && route !== live.state.route && this.#isLive(live)) {
        this.#update(live, { route }, true);
      }
    } catch {
      // A closed connection has no stats; its last route stands.
    }
  }

  #isLive(live: Live): boolean {
    return this.#live.get(key(live.state.offer, live.state.peer)) === live;
  }

  #fail(live: Live): void {
    if (!this.#isLive(live)) {
      return;
    }
    this.#send({
      type: "endTransfer",
      offer: live.state.offer,
      peer: live.state.peer,
      reason: "failed",
    });
    this.#end(live, "failed", "self");
  }

  /** Closes a transfer at once and keeps its final state for the UI. */
  #end(live: Live, status: TransferEnd, endedBy: "self" | "peer", file: Blob | null = null): void {
    if (!this.#isLive(live)) {
      return;
    }
    const k = key(live.state.offer, live.state.peer);
    this.#live.delete(k);
    if (live.routeTimer !== null) {
      clearInterval(live.routeTimer);
    }
    live.channel?.close();
    live.pc.close();
    live.received = [];
    if (status !== "completed") {
      // What was written of a file that did not finish is discarded.
      void live.sink?.abort().catch(() => undefined);
    }
    this.#finished.set(k, { ...live.state, status, endedBy, file });
    this.#releaseFile(live.state.offer);
    this.#changed();
  }

  /** Lets go of an offered file once its offer is gone and nothing is sending it. */
  #releaseFile(offer: string): void {
    const sending = [...this.#live.values()].some(
      (live) => live.state.offer === offer && live.state.role === "sender",
    );
    if (!this.#offers.has(offer) && !sending) {
      this.#files.delete(offer);
    }
  }

  #update(live: Live, patch: Partial<TransferState>, force = false): void {
    live.state = { ...live.state, ...patch };
    const now = this.#now();
    if (force || now - live.lastReport >= PROGRESS_INTERVAL_MS) {
      live.lastReport = now;
      this.#changed();
    }
  }

  #changed(): void {
    this.#snapshot = {
      offers: [...this.#offers.values()],
      transfers: [
        ...[...this.#live.values()].map((live) => live.state),
        ...this.#finished.values(),
      ],
      links: this.#links,
      relayMbps: this.#relayMbps,
    };
    this.#onChange();
  }
}
