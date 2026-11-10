import type { AspenClient, UploadJob } from "@aspen/protocol";
import { beforeEach, describe, expect, it, vi } from "vitest";

/** What the native side was asked, in order, and what it answers `list` with. */
const calls: { method: string; options: Record<string, unknown> }[] = [];
let held: Record<string, unknown>[] = [];
const listeners = new Map<string, (event: Record<string, unknown>) => void>();

vi.mock("@capacitor/core", () => ({
  Capacitor: {
    isNativePlatform: () => true,
    isPluginAvailable: (name: string) => name === "AspenUploads",
  },
  registerPlugin: () =>
    new Proxy(
      {},
      {
        get: (_, method: string) => {
          if (method === "addListener") {
            return (event: string, listener: (event: Record<string, unknown>) => void) => {
              listeners.set(event, listener);
              return Promise.resolve({ remove: () => Promise.resolve() });
            };
          }
          return (options: Record<string, unknown>) => {
            calls.push({ method, options });
            return Promise.resolve(method === "list" ? { uploads: held } : undefined);
          };
        },
      },
    ),
}));

const { UploadDiscardedError, ApiProblemError } = await import("@aspen/protocol");
const { nativeSendsUploads, nativeUploadTransport, wordNativeUploads } =
  await import("./uploadsBridge");

const client = {
  baseUrl: "https://chat.example",
  session: {
    userId: "user-1",
    sessionToken: "session",
    refreshToken: "refresh",
    sessionTokenExpires: "2030-01-01T00:00:00Z",
  },
} as unknown as AspenClient;

function job(overrides: Partial<UploadJob> = {}): UploadJob {
  return {
    attachmentId: "a1",
    uploadUrl: "https://store.example/put/a1",
    contentType: "text/plain",
    name: "notes.txt",
    mimeType: "text/plain",
    byteSize: 5,
    file: new Blob(["hello"]),
    onProgress: () => undefined,
    signal: new AbortController().signal,
    ...overrides,
  };
}

/** Lets the bridge's awaited calls run to the next one that waits on the native side. */
async function settle(): Promise<void> {
  for (let i = 0; i < 20; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

const record = {
  id: "a1",
  fileName: "notes.txt",
  mimeType: "text/plain",
  downloadUrl: "https://m",
};

describe("uploadsBridge", () => {
  beforeEach(() => {
    calls.length = 0;
    held = [];
  });

  it("is used in a mobile app that has the plugin", () => {
    expect(nativeSendsUploads()).toBe(true);
  });

  it("hands over the bytes, then where they go and whom to confirm as", async () => {
    wordNativeUploads({
      pending: {
        notifyFailedTitle: "Message not sent",
        notifyFailedBody: "{name} could not be uploaded.",
        notifySending: "Sending your files…",
      },
    } as never);
    const progress: number[] = [];
    const sent = nativeUploadTransport(client).send(
      job({ onProgress: (bytes) => progress.push(bytes) }),
    );
    await settle();
    expect(calls.map((call) => call.method).slice(0, 3)).toEqual(["begin", "write", "send"]);
    expect(atob(calls[1]?.options.data as string)).toBe("hello");
    expect(calls[2]?.options).toEqual({
      id: "a1",
      origin: "https://chat.example",
      userId: "user-1",
      sessionToken: "session",
      refreshToken: "refresh",
      uploadUrl: "https://store.example/put/a1",
      contentType: "text/plain",
      name: "notes.txt",
      mimeType: "text/plain",
      byteSize: 5,
      acceptLanguage: "",
      failedTitle: "Message not sent",
      failedBody: "notes.txt could not be uploaded.",
      sendingTitle: "Sending your files…",
    });
    listeners.get("progress")?.({ id: "a1", sent: 3, total: 5 });
    expect(progress).toEqual([3]);
    listeners.get("settled")?.({ id: "a1", state: "done", attachment: JSON.stringify(record) });
    expect(await sent).toEqual(record);
    // The record is the page's now, and the native side keeps nothing of it.
    expect(calls.at(-1)).toEqual({ method: "discard", options: { id: "a1" } });
  });

  it("sends again from the bytes the native side kept, when the page holds none", async () => {
    const sent = nativeUploadTransport(client).send(job({ file: null }));
    await settle();
    expect(calls.map((call) => call.method)).not.toContain("begin");
    expect(calls.map((call) => call.method)).toContain("send");
    listeners.get("settled")?.({ id: "a1", state: "done", attachment: JSON.stringify(record) });
    await sent;
  });

  it("throws a refusal in the deployment's words, and anything else as unreachable", async () => {
    const transport = nativeUploadTransport(client);
    const refused = transport.send(job());
    await settle();
    listeners.get("settled")?.({
      id: "a1",
      state: "failed",
      cause: "refused",
      reason: "The file is larger than this deployment allows.",
    });
    await expect(refused).rejects.toBeInstanceOf(ApiProblemError);
    await expect(refused).rejects.toThrow("The file is larger than this deployment allows.");

    const lost = transport.send(job());
    await settle();
    listeners.get("settled")?.({ id: "a1", state: "failed", cause: "unreachable", reason: null });
    await expect(lost).rejects.toBeInstanceOf(TypeError);
  });

  it("learns what ended while the page slept, when the app comes back to the front", async () => {
    const sent = nativeUploadTransport(client).send(job());
    await settle();
    held = [
      {
        id: "a1",
        name: "notes.txt",
        mimeType: "text/plain",
        byteSize: 5,
        state: "done",
        sent: 5,
        attachment: JSON.stringify(record),
      },
    ];
    document.dispatchEvent(new Event("visibilitychange"));
    expect(await sent).toEqual(record);
  });

  it("stops a discarded upload on the native side", async () => {
    const abort = new AbortController();
    const sent = nativeUploadTransport(client).send(job({ signal: abort.signal }));
    await settle();
    abort.abort();
    await expect(sent).rejects.toBeInstanceOf(UploadDiscardedError);
    expect(calls.at(-1)).toEqual({ method: "discard", options: { id: "a1" } });
  });

  it("finds what the native side still holds when the page loads, and lets go of what arrived", async () => {
    held = [
      {
        id: "a2",
        name: "clip.mp4",
        mimeType: "video/mp4",
        byteSize: 900,
        state: "sending",
        sent: 10,
      },
      { id: "a3", name: "old.txt", mimeType: "text/plain", byteSize: 4, state: "failed", sent: 0 },
      {
        id: "a4",
        name: "done.txt",
        mimeType: "text/plain",
        byteSize: 4,
        state: "done",
        sent: 4,
        attachment: "{}",
      },
    ];
    const kept = await nativeUploadTransport(client).kept?.();
    expect(calls[0]).toEqual({
      method: "list",
      options: { origin: "https://chat.example", userId: "user-1" },
    });
    expect(kept).toEqual([
      { attachmentId: "a2", name: "clip.mp4", mimeType: "video/mp4", byteSize: 900 },
      { attachmentId: "a3", name: "old.txt", mimeType: "text/plain", byteSize: 4 },
    ]);
    expect(calls.filter((call) => call.method === "discard")).toEqual([
      { method: "discard", options: { id: "a4" } },
    ]);
  });

  it("follows a kept upload to its end", async () => {
    held = [
      {
        id: "a2",
        name: "clip.mp4",
        mimeType: "video/mp4",
        byteSize: 900,
        state: "sending",
        sent: 450,
      },
    ];
    const progress: number[][] = [];
    const followed = nativeUploadTransport(client).follow?.(
      "a2",
      (sent, total) => progress.push([sent, total]),
      new AbortController().signal,
    );
    await settle();
    // How far it had come when the page asked.
    expect(progress).toEqual([[450, 900]]);
    listeners.get("settled")?.({ id: "a2", state: "failed", cause: "unreachable", reason: null });
    await expect(followed).rejects.toBeInstanceOf(TypeError);
  });
});
