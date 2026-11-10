import { describe, expect, it } from "vitest";
import {
  AspenClient,
  AttachmentUploads,
  MemorySessionStore,
  RecordStore,
  UploadDiscardedError,
  type Attachment,
  type KeptUpload,
  type UploadJob,
  type UploadTransport,
} from "../src";

const baseUrl = "https://aspen.test";

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function record(id: string, fileName = "note.txt"): Attachment {
  return {
    id,
    fileName,
    mimeType: "text/plain",
    downloadUrl: `https://files.test/attachments/${id}`,
  };
}

/** A transport whose sends wait for the test to end them. */
function heldTransport() {
  const jobs: (UploadJob & {
    finish: (attachment: Attachment) => void;
    fail: (e: unknown) => void;
  })[] = [];
  const discarded: string[] = [];
  const transport: UploadTransport = {
    send: (job) =>
      new Promise<Attachment>((finish, fail) => {
        jobs.push({ ...job, finish, fail });
        job.signal.addEventListener("abort", () => {
          fail(new UploadDiscardedError());
        });
      }),
    discard: (attachmentId) => {
      discarded.push(attachmentId);
    },
  };
  return { transport, jobs, discarded };
}

function memoryStorage() {
  const kept = new Map<string, string>();
  return {
    getItem: (key: string) => kept.get(key) ?? null,
    setItem: (key: string, value: string) => void kept.set(key, value),
    removeItem: (key: string) => void kept.delete(key),
  };
}

/** Uploads over a server that reserves attachments `a1`, `a2`, … and records what it is asked. */
function makeUploads(transport: UploadTransport, storage = memoryStorage()) {
  const requests: { method: string; path: string; body: unknown }[] = [];
  let reserved = 0;
  let refuseReservations = false;
  const sessions = new MemorySessionStore();
  sessions.save({
    userId: "user-1",
    refreshToken: "refresh",
    sessionToken: "session",
    sessionTokenExpires: new Date(Date.now() + 3_600_000).toISOString(),
  });
  const client = new AspenClient({
    baseUrl,
    sessionStore: sessions,
    fetch: async (input, init) => {
      const request = new Request(input, init);
      const path = new URL(request.url).pathname;
      const body: unknown = request.method === "DELETE" ? null : await request.json();
      requests.push({ method: request.method, path, body });
      if (request.method === "POST" && path === "/api/v1/attachments") {
        if (refuseReservations) {
          return new Response(
            JSON.stringify({
              code: "validation",
              title: "Refused",
              detail: "Too large.",
              status: 400,
            }),
            { status: 400, headers: { "content-type": "application/problem+json" } },
          );
        }
        reserved += 1;
        return json(
          {
            id: `a${String(reserved)}`,
            uploadUrl: `https://store.test/put/a${String(reserved)}`,
            expiresAt: "2030-01-01T00:00:00Z",
            contentType: "text/plain",
          },
          201,
        );
      }
      if (request.method === "POST" && path.endsWith("/renew")) {
        const id = path.split("/").at(-2) ?? "";
        return json({
          id,
          uploadUrl: `https://store.test/put/${id}?again`,
          expiresAt: "2030-01-01T00:00:00Z",
          contentType: "text/plain",
        });
      }
      if (request.method === "PATCH") {
        return json(record(path.split("/").at(-1) ?? ""));
      }
      return new Response(null, { status: 204 });
    },
  });
  const store = new RecordStore({ now: () => 0 });
  const uploads = new AttachmentUploads({ client, store, transport, storage });
  return {
    uploads,
    store,
    requests,
    storage,
    refuse: () => {
      refuseReservations = true;
    },
  };
}

async function settle(): Promise<void> {
  for (let i = 0; i < 4; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

const file = (name = "note.txt") => new File(["hello"], name, { type: "text/plain" });

describe("AttachmentUploads", () => {
  it("reserves, sends, and leaves once the record is confirmed", async () => {
    const { transport, jobs } = heldTransport();
    const { uploads, store } = makeUploads(transport);
    const { key } = uploads.add(file());
    expect(uploads.get(key)?.state).toEqual({ kind: "uploading", sent: 0 });
    expect(await uploads.reserved(key)).toBe("a1");
    await settle();
    expect(jobs[0]).toMatchObject({
      attachmentId: "a1",
      uploadUrl: "https://store.test/put/a1",
      contentType: "text/plain",
      byteSize: 5,
    });
    jobs[0]?.onProgress(2, 4);
    expect(uploads.byAttachment("a1")?.state).toEqual({ kind: "uploading", sent: 0.5 });
    jobs[0]?.finish(record("a1"));
    expect(await uploads.settled(key)).toEqual(record("a1"));
    expect(uploads.all()).toEqual([]);
    expect(store.attachment("a1")).toEqual(record("a1"));
  });

  it("tells subscribers of each change with a new list", async () => {
    const { transport, jobs } = heldTransport();
    const { uploads } = makeUploads(transport);
    const seen: number[] = [];
    const stop = uploads.subscribe(() => seen.push(uploads.all().length));
    const before = uploads.all();
    uploads.add(file());
    expect(uploads.all()).not.toBe(before);
    await settle();
    jobs[0]?.finish(record("a1"));
    await settle();
    stop();
    expect(seen.at(0)).toBe(1);
    expect(seen.at(-1)).toBe(0);
  });

  it("keeps a failed upload, and sends it again to a renewed link for the same attachment", async () => {
    const { transport, jobs } = heldTransport();
    const { uploads, requests } = makeUploads(transport);
    const { key } = uploads.add(file());
    const settled = uploads.settled(key);
    await settle();
    jobs[0]?.fail(new Error("the upload could not reach storage"));
    await settle();
    expect(uploads.get(key)?.state).toEqual({
      kind: "failed",
      cause: "unreachable",
      reason: null,
      retriable: true,
    });
    uploads.retry(key);
    expect(uploads.get(key)?.state).toEqual({ kind: "uploading", sent: 0 });
    await settle();
    expect(requests.at(-1)).toEqual({
      method: "POST",
      path: "/api/v1/attachments/a1/renew",
      body: { byteSize: 5 },
    });
    expect(jobs[1]).toMatchObject({
      attachmentId: "a1",
      uploadUrl: "https://store.test/put/a1?again",
    });
    jobs[1]?.finish(record("a1"));
    // Whoever waited for the first try has the second's record.
    expect(await settled).toEqual(record("a1"));
  });

  it("reserves again an upload whose reservation was refused", async () => {
    const { transport, jobs } = heldTransport();
    const made = makeUploads(transport);
    made.refuse();
    const { key } = made.uploads.add(file());
    await expect(made.uploads.reserved(key)).rejects.toThrow("Too large.");
    await settle();
    expect(made.uploads.get(key)?.state).toMatchObject({
      kind: "failed",
      cause: "refused",
      reason: "Too large.",
    });
    expect(made.uploads.get(key)?.attachmentId).toBeNull();
    expect(jobs).toEqual([]);
  });

  it("gives the server a change made before the attachment was reserved, once it is", async () => {
    const { transport } = heldTransport();
    const { uploads, requests } = makeUploads(transport);
    const { key } = uploads.add(file());
    await uploads.change(key, { description: "A note" });
    await uploads.change(key, { spoiler: true });
    await settle();
    expect(requests.filter((r) => r.method === "PATCH")).toEqual([
      {
        method: "PATCH",
        path: "/api/v1/attachments/a1",
        body: { description: "A note", spoiler: true },
      },
    ]);
    await uploads.change(key, { spoiler: false });
    expect(requests.at(-1)?.body).toEqual({ spoiler: false });
  });

  it("stops a discarded upload, deletes its reservation, and rejects whoever waited", async () => {
    const { transport, jobs, discarded } = heldTransport();
    const { uploads, requests } = makeUploads(transport);
    const { key } = uploads.add(file());
    const settled = uploads.settled(key);
    await settle();
    uploads.discard(key);
    await expect(settled).rejects.toBeInstanceOf(UploadDiscardedError);
    expect(jobs[0]?.signal.aborted).toBe(true);
    expect(uploads.all()).toEqual([]);
    expect(discarded).toEqual(["a1"]);
    await settle();
    expect(requests.at(-1)).toMatchObject({ method: "DELETE", path: "/api/v1/attachments/a1" });
  });

  it("says whether a sent message still waits for an upload on its way", async () => {
    const { transport, jobs } = heldTransport();
    const { uploads } = makeUploads(transport);
    const { key } = uploads.add(file());
    await settle();
    expect(uploads.sending).toBe(false);
    uploads.detach([key]);
    expect(uploads.sending).toBe(true);
    uploads.attach([key]);
    expect(uploads.sending).toBe(false);
    uploads.detach([key]);
    jobs[0]?.finish(record("a1"));
    await settle();
    expect(uploads.sending).toBe(false);
  });

  it("shows an upload a sent message waited for as interrupted on the next page", async () => {
    const first = heldTransport();
    const before = makeUploads(first.transport);
    const { key } = before.uploads.add(file("photo.png"));
    await settle();
    before.uploads.detach([key]);

    const after = makeUploads(heldTransport().transport, before.storage);
    await after.uploads.restore();
    expect(after.uploads.byAttachment("a1")).toMatchObject({
      name: "photo.png",
      file: null,
      state: { kind: "failed", cause: "interrupted", reason: null, retriable: false },
    });
    // One still in a message box when the page went is nobody's concern.
    expect(after.uploads.all()).toHaveLength(1);
  });

  it("notes nothing of an upload once it has arrived", async () => {
    const { transport, jobs } = heldTransport();
    const before = makeUploads(transport);
    const { key } = before.uploads.add(file());
    await settle();
    before.uploads.detach([key]);
    jobs[0]?.finish(record("a1"));
    await settle();
    const after = makeUploads(heldTransport().transport, before.storage);
    await after.uploads.restore();
    expect(after.uploads.all()).toEqual([]);
  });

  it("watches what the transport kept going across a page load to its end", async () => {
    const kept: KeptUpload[] = [
      { attachmentId: "a7", name: "clip.mp4", mimeType: "video/mp4", byteSize: 900 },
    ];
    let finish: (attachment: Attachment) => void = () => undefined;
    const progress: ((sent: number, total: number) => void)[] = [];
    const transport: UploadTransport = {
      send: () => Promise.reject(new Error("not sent here")),
      kept: () => Promise.resolve(kept),
      follow: (_id, onProgress) => {
        progress.push(onProgress);
        return new Promise<Attachment>((resolve) => {
          finish = resolve;
        });
      },
    };
    const { uploads, store } = makeUploads(transport);
    await uploads.restore();
    expect(uploads.byAttachment("a7")).toMatchObject({ name: "clip.mp4", file: null });
    progress[0]?.(450, 900);
    expect(uploads.byAttachment("a7")?.state).toEqual({ kind: "uploading", sent: 0.5 });
    expect(uploads.sending).toBe(true);
    finish(record("a7", "clip.mp4"));
    await settle();
    expect(uploads.all()).toEqual([]);
    expect(store.attachment("a7")?.fileName).toBe("clip.mp4");
  });

  it("lets go of uploads the server no longer waits for, whichever is learned first", async () => {
    const discarded: string[] = [];
    const transport: UploadTransport = {
      send: () => Promise.reject(new Error("not sent here")),
      kept: () =>
        Promise.resolve([
          { attachmentId: "a1", name: "one", mimeType: "text/plain", byteSize: 1 },
          { attachmentId: "a2", name: "two", mimeType: "text/plain", byteSize: 1 },
        ]),
      follow: () => new Promise<Attachment>(() => undefined),
      discard: (id) => void discarded.push(id),
    };
    const { uploads } = makeUploads(transport);
    // The server's answer arrives before the transport's.
    uploads.retain(new Set(["a2"]));
    await uploads.restore();
    expect(uploads.all().map((upload) => upload.attachmentId)).toEqual(["a2"]);
    expect(discarded).toEqual(["a1"]);
    uploads.retain(new Set());
    expect(uploads.all()).toEqual([]);
    expect(discarded).toEqual(["a1", "a2"]);
  });

  it("leaves an upload still in a message box alone when the server waits for none", async () => {
    const { transport } = heldTransport();
    const { uploads } = makeUploads(transport);
    uploads.add(file());
    await settle();
    uploads.retain(new Set());
    expect(uploads.all()).toHaveLength(1);
  });
});
