import { describe, expect, it } from "vitest";
import type { ClientMessage, ServerMessage } from "./generated/voiceSignal";
import { FileTransfers, routeOf, type TransferState } from "./transfers";

/** A data channel whose other end is another fake's, delivering in order on the next tick. */
class FakeChannel {
  /** When set, nothing sent is delivered and the buffer only grows, as on a stalled link. */
  static stalled = false;
  peer: FakeChannel | null = null;
  readyState: RTCDataChannelState = "connecting";
  binaryType: BinaryType = "blob";
  bufferedAmount = 0;
  bufferedAmountLowThreshold = 0;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onbufferedamountlow: (() => void) | null = null;
  sent = 0;

  open(): void {
    this.readyState = "open";
    this.onopen?.();
  }

  send(data: ArrayBuffer | string): void {
    const size = typeof data === "string" ? data.length : data.byteLength;
    this.sent += size;
    if (FakeChannel.stalled) {
      this.bufferedAmount += size;
      return;
    }
    const peer = this.peer;
    queueMicrotask(() => {
      peer?.onmessage?.({ data } as MessageEvent);
    });
  }

  close(): void {
    if (this.readyState === "closed") {
      return;
    }
    this.readyState = "closed";
    const peer = this.peer;
    queueMicrotask(() => {
      if (peer !== null && peer.readyState !== "closed") {
        peer.readyState = "closed";
        peer.onclose?.();
      }
    });
  }
}

/** A peer connection that connects to its partner once both descriptions are set. */
class FakePeerConnection {
  static made: FakePeerConnection[] = [];
  readonly configuration: RTCConfiguration;
  localDescription: { type: string; toJSON(): RTCSessionDescriptionInit } | null = null;
  remoteDescription: RTCSessionDescriptionInit | null = null;
  connectionState = "new";
  sctp = { maxMessageSize: 262144 };
  onicecandidate: ((event: { candidate: null }) => void) | null = null;
  onconnectionstatechange: (() => void) | null = null;
  ondatachannel: ((event: { channel: FakeChannel }) => void) | null = null;
  channel: FakeChannel | null = null;
  partner: FakePeerConnection | null = null;
  closed = false;

  constructor(configuration: RTCConfiguration) {
    this.configuration = configuration;
    FakePeerConnection.made.push(this);
  }

  createDataChannel(): FakeChannel {
    this.channel = new FakeChannel();
    return this.channel;
  }

  createOffer(): Promise<RTCSessionDescriptionInit> {
    return Promise.resolve({ type: "offer", sdp: "fake" });
  }

  createAnswer(): Promise<RTCSessionDescriptionInit> {
    return Promise.resolve({ type: "answer", sdp: "fake" });
  }

  setLocalDescription(description: RTCSessionDescriptionInit): Promise<void> {
    this.localDescription = { type: description.type, toJSON: () => description };
    return Promise.resolve();
  }

  setRemoteDescription(description: RTCSessionDescriptionInit): Promise<void> {
    this.remoteDescription = description;
    if (description.type === "answer") {
      // The offerer hears the answer: connect the two, the answerer's channel mirroring ours.
      const answerer = FakePeerConnection.made.find(
        (pc) => pc !== this && pc.remoteDescription?.type === "offer" && pc.partner === null,
      );
      if (answerer !== undefined && this.channel !== null) {
        this.partner = answerer;
        answerer.partner = this;
        const mirror = new FakeChannel();
        mirror.peer = this.channel;
        this.channel.peer = mirror;
        queueMicrotask(() => {
          for (const pc of [this, answerer]) {
            pc.connectionState = "connected";
            pc.onconnectionstatechange?.();
          }
          answerer.ondatachannel?.({ channel: mirror });
          mirror.open();
          this.channel?.open();
        });
      }
    }
    return Promise.resolve();
  }

  addIceCandidate(): Promise<void> {
    return Promise.resolve();
  }

  /** The selected pair as Chromium reports it: relay candidates when only the relay was allowed. */
  getStats(): Promise<Map<string, object>> {
    const type = this.configuration.iceTransportPolicy === "relay" ? "relay" : "host";
    return Promise.resolve(
      new Map<string, object>([
        ["T", { type: "transport", selectedCandidatePairId: "P" }],
        ["P", { type: "candidate-pair", localCandidateId: "L", remoteCandidateId: "R" }],
        ["L", { type: "local-candidate", candidateType: type }],
        ["R", { type: "remote-candidate", candidateType: type }],
      ]),
    );
  }

  close(): void {
    this.closed = true;
    this.channel?.close();
  }
}

const SENDER = "00000000-0000-0000-0000-00000000000a";
const RECEIVER = "00000000-0000-0000-0000-00000000000b";
const OFFER = "00000000-0000-0000-0000-0000000000f1";

/** Two devices in one call, and a voice server between them that routes the way the real one does. */
function call(relayMbps: number | null = 50) {
  FakePeerConnection.made = [];
  FakeChannel.stalled = false;
  const devices = new Map<string, FileTransfers>();
  const tell = (user: string, frame: ServerMessage): void => {
    devices.get(user)?.handle(frame);
  };
  const everyone = (frame: ServerMessage): void => {
    for (const user of devices.keys()) {
      tell(user, frame);
    }
  };
  const offers = new Map<
    string,
    { from: string; name: string; size: number; allowDirect: boolean }
  >();
  const serve = (user: string, frame: ClientMessage): void => {
    queueMicrotask(() => {
      switch (frame.type) {
        case "offerFile":
          offers.set(frame.offer, {
            from: user,
            name: frame.name,
            size: frame.size,
            allowDirect: frame.allowDirect,
          });
          everyone({
            type: "fileOffered",
            offer: {
              id: frame.offer,
              from: user,
              name: frame.name,
              size: frame.size,
              allowDirect: frame.allowDirect,
              expiresInMs: frame.validForSeconds * 1000,
            },
          });
          break;
        case "acceptFile": {
          const offer = offers.get(frame.offer);
          if (offer === undefined) {
            break;
          }
          const ice = [
            { urls: ["turn:voice.example:3478?transport=udp"], username: "u", credential: "c" },
          ];
          tell(offer.from, {
            type: "transferStarting",
            offer: frame.offer,
            peer: user,
            role: "sender",
            mode: frame.mode,
            name: offer.name,
            size: offer.size,
            iceServers: ice,
          });
          tell(user, {
            type: "transferStarting",
            offer: frame.offer,
            peer: offer.from,
            role: "receiver",
            mode: frame.mode,
            name: offer.name,
            size: offer.size,
            iceServers: ice,
          });
          everyone({
            type: "transferLinkChanged",
            link: { sender: offer.from, receiver: user },
            active: true,
          });
          break;
        }
        case "transferSignal":
          tell(frame.peer, {
            type: "transferSignal",
            offer: frame.offer,
            peer: user,
            signal: frame.signal,
          });
          break;
        case "endTransfer": {
          tell(frame.peer, {
            type: "transferEnded",
            offer: frame.offer,
            peer: user,
            reason: frame.reason,
          });
          const sender = offers.get(frame.offer)?.from ?? user;
          everyone({
            type: "transferLinkChanged",
            link: { sender, receiver: sender === user ? frame.peer : user },
            active: false,
          });
          break;
        }
        default:
          break;
      }
    });
  };
  for (const user of [SENDER, RECEIVER]) {
    const device = new FileTransfers({
      send: (frame) => {
        serve(user, frame);
      },
      onChange: () => undefined,
      createPeerConnection: (configuration) =>
        new FakePeerConnection(configuration) as unknown as RTCPeerConnection,
      randomId: () => OFFER,
    });
    devices.set(user, device);
    device.reset({
      type: "ready",
      session: "00000000-0000-0000-0000-000000000005",
      user,
      routerRtpCapabilities: {},
      participants: [],
      offers: [],
      links: [],
      transfers: { relayMbps },
    });
  }
  const sender = devices.get(SENDER);
  const receiver = devices.get(RECEIVER);
  if (sender === undefined || receiver === undefined) {
    throw new Error("devices missing");
  }
  return { sender, receiver };
}

/** Lets queued deliveries and awaited promises run until nothing is left. */
async function settle(): Promise<void> {
  for (let i = 0; i < 200; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

function transferOf(state: { transfers: readonly TransferState[] }): TransferState | undefined {
  return state.transfers[0];
}

describe("FileTransfers", () => {
  it("sends an offered file to whoever accepts it", async () => {
    const { sender, receiver } = call();
    const bytes = new Uint8Array(200_000).map((_, i) => i % 251);
    sender.offer(new Blob([bytes]), "notes.bin", true, 60);
    await settle();
    expect(receiver.state.offers).toMatchObject([
      { id: OFFER, from: SENDER, name: "notes.bin", size: 200_000, allowDirect: true, own: false },
    ]);
    expect(sender.state.offers[0]?.own).toBe(true);

    receiver.accept(OFFER, "directPreferred");
    await settle();

    const received = transferOf(receiver.state);
    expect(received).toMatchObject({ role: "receiver", status: "completed", bytes: 200_000 });
    expect(new Uint8Array(await (received?.file ?? new Blob()).arrayBuffer())).toEqual(bytes);
    expect(transferOf(sender.state)).toMatchObject({
      role: "sender",
      status: "completed",
      endedBy: "peer",
    });
    expect(sender.state.links).toEqual([]);
  });

  it("relays only when the receiver chose relayOnly", async () => {
    const { sender, receiver } = call();
    sender.offer(new Blob(["x"]), "a.txt", true, 60);
    await settle();
    receiver.accept(OFFER, "relayOnly");
    await settle();
    expect(FakePeerConnection.made.map((pc) => pc.configuration.iceTransportPolicy)).toEqual([
      "relay",
      "relay",
    ]);
    expect(transferOf(receiver.state)).toMatchObject({ mode: "relayOnly", route: "relayed" });
  });

  it("closes both sides at once when either cancels", async () => {
    const { sender, receiver } = call();
    sender.offer(new Blob([new Uint8Array(50_000_000)]), "big.bin", true, 60);
    await settle();
    FakeChannel.stalled = true;
    receiver.accept(OFFER, "directPreferred");
    await settle();
    expect(transferOf(sender.state)).toMatchObject({ status: "moving" });
    receiver.cancel(OFFER, SENDER);
    expect(transferOf(receiver.state)).toMatchObject({ status: "cancelled", endedBy: "self" });
    await settle();
    expect(transferOf(sender.state)).toMatchObject({ status: "cancelled", endedBy: "peer" });
    expect(FakePeerConnection.made.every((pc) => pc.closed)).toBe(true);
  });

  it("fails a transfer that brings more than was offered", async () => {
    const { sender, receiver } = call();
    // A file that states four bytes and holds ten.
    class Lying extends Blob {
      override get size(): number {
        return 4;
      }
    }
    sender.offer(new Lying(["0123456789"]), "ten.txt", true, 60);
    await settle();
    receiver.accept(OFFER, "directPreferred");
    await settle();
    expect(transferOf(receiver.state)).toMatchObject({ status: "failed" });
  });

  it("keeps a transfer going after its offer is withdrawn", async () => {
    const { sender, receiver } = call();
    sender.offer(new Blob([new Uint8Array(100_000)]), "keep.bin", true, 60);
    await settle();
    receiver.accept(OFFER, "directPreferred");
    // The server tells the sender a transfer is starting before any later withdrawal.
    while (transferOf(sender.state) === undefined) {
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    sender.handle({ type: "fileWithdrawn", offer: OFFER, reason: "expired" });
    receiver.handle({ type: "fileWithdrawn", offer: OFFER, reason: "expired" });
    await settle();
    expect(receiver.state.offers).toEqual([]);
    expect(transferOf(receiver.state)).toMatchObject({ status: "completed", bytes: 100_000 });
  });

  it("reads the route of the selected pair in either browser's shape", async () => {
    const pc = (entries: [string, object][]) =>
      ({ getStats: () => Promise.resolve(new Map(entries)) }) as unknown as RTCPeerConnection;
    // Chromium: the transport names the pair.
    expect(
      await routeOf(
        pc([
          ["T", { type: "transport", selectedCandidatePairId: "P" }],
          ["P", { type: "candidate-pair", localCandidateId: "L", remoteCandidateId: "R" }],
          ["L", { candidateType: "srflx" }],
          ["R", { candidateType: "relay" }],
        ]),
      ),
    ).toBe("relayed");
    // Firefox: the pair is marked nominated.
    expect(
      await routeOf(
        pc([
          [
            "Q",
            {
              type: "candidate-pair",
              nominated: false,
              state: "succeeded",
              localCandidateId: "X",
              remoteCandidateId: "Y",
            },
          ],
          [
            "P",
            {
              type: "candidate-pair",
              nominated: true,
              state: "succeeded",
              localCandidateId: "L",
              remoteCandidateId: "R",
            },
          ],
          ["L", { candidateType: "host" }],
          ["R", { candidateType: "srflx" }],
        ]),
      ),
    ).toBe("direct");
    expect(await routeOf(pc([]))).toBeNull();
  });
});
