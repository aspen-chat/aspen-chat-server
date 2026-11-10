/**
 * Files on their way to the server for messages (`AttachmentUploads`). An upload belongs to the
 * session, not to the message box that began it: a message may be sent while its files are
 * still uploading (`awaitUploads`), and the uploads then go on with no box holding them, the
 * server posting the message as the last is confirmed.
 *
 * The bytes travel by an `UploadTransport`. A browser's sends them itself and ends with the
 * page; a phone's native one keeps sending while the app is in the background and confirms
 * each upload itself, so what it was sending when the page last went is found again here
 * (`restore`).
 */

import type { components } from "./generated/openapi";
import { type AspenClient, problemOf } from "./http";
import type { PreferenceStorage } from "./preferences";
import { ApiProblemError, transportProblem } from "./problem";
import type { RecordStore } from "./store";
import type { AttachmentChange } from "./upload";
import { requireSecure } from "./urls";

type Attachment = components["schemas"]["Attachment"];

/** How an upload stands. */
export type UploadState =
  /** `sent` is how much of the file has reached storage, 0 to 1. */
  | { readonly kind: "uploading"; readonly sent: number }
  | {
      readonly kind: "failed";
      /**
       * `refused`: the server would not take it, and `reason` is why, in its words.
       * `unreachable`: the bytes did not get to storage, or the server did not answer.
       * `interrupted`: the page that was sending it went away.
       */
      readonly cause: "refused" | "unreachable" | "interrupted";
      /** The server's own words for a refusal, in the reader's language; `null` otherwise. */
      readonly reason: string | null;
      /** Whether `retry` can send it again: the bytes are still held, here or by the transport. */
      readonly retriable: boolean;
    };

/**
 * A file on its way to the server, or one that failed to get there. One that arrives leaves
 * the uploads: its record is then the store's (`RecordStore.attachment`), and whoever waited
 * for it has it from `settled`.
 */
export interface Upload {
  /** Names it on this device, before the server has given it an id and after. */
  readonly key: string;
  readonly name: string;
  readonly mimeType: string;
  readonly byteSize: number;
  /** The attachment it was reserved as, once the server has answered. */
  readonly attachmentId: string | null;
  /** The bytes, while this page holds them: for showing a picture before it has arrived. */
  readonly file: Blob | null;
  /** Whether a sent message waits for it, with no message box holding it (`detach`). */
  readonly detached: boolean;
  readonly state: UploadState;
}

/** One file for a transport to send and confirm. */
export interface UploadJob {
  readonly attachmentId: string;
  /** Where the bytes go: the presigned URL of the upload's handle, good until it expires. */
  readonly uploadUrl: string;
  /** The `Content-Type` the URL is signed for. */
  readonly contentType: string;
  readonly name: string;
  readonly mimeType: string;
  readonly byteSize: number;
  /**
   * The bytes; `null` for an upload the transport kept from before the page loaded
   * (`UploadTransport.kept`), which holds them itself.
   */
  readonly file: Blob | null;
  readonly onProgress: (sent: number, total: number) => void;
  /** Aborted when the upload is discarded: the transport stops sending and rejects. */
  readonly signal: AbortSignal;
}

/** An upload a transport was still sending, or had failed to send, when the page last went. */
export interface KeptUpload {
  readonly attachmentId: string;
  readonly name: string;
  readonly mimeType: string;
  readonly byteSize: number;
}

/** What carries an upload's bytes to storage and confirms it with the server. */
export interface UploadTransport {
  /**
   * Sends `job`'s bytes to its URL and confirms the upload, answering the confirmed record.
   * Rejects with an `ApiProblemError` the user can read when either step fails.
   */
  send(job: UploadJob): Promise<Attachment>;
  /**
   * The uploads it kept going, or kept the bytes of, across a page load: a transport that
   * outlives the page has one. Each is then watched to its end with `follow`.
   */
  kept?(): Promise<readonly KeptUpload[]>;
  /** Watches a kept upload to its end, as `send` ends: confirmed, or failed. */
  follow?(
    attachmentId: string,
    onProgress: (sent: number, total: number) => void,
    signal: AbortSignal,
  ): Promise<Attachment>;
  /** Lets go of whatever it keeps of an upload that will never be sent again. */
  discard?(attachmentId: string): void;
}

export interface AttachmentUploadsOptions {
  client: AspenClient;
  store: RecordStore;
  transport: UploadTransport;
  /**
   * Where the uploads a sent message waits for are noted, so a page loaded after the one
   * sending them went away can say they were interrupted; `null` notes nothing.
   */
  storage?: PreferenceStorage | null;
}

/** What `storage` keeps of an upload a sent message waits for. */
interface Noted extends KeptUpload {
  /** The account it is for, so one signed in here later is not shown another's. */
  readonly userId: string;
}

const STORAGE_KEY = "aspen.uploads.detached";

interface Entry {
  /** As others read it; `detached` says whether a sent message waits for it. */
  upload: Upload;
  abort: AbortController;
  /** A picture's size in pixels, declared when the upload is reserved. */
  size: { readonly width: number; readonly height: number } | undefined;
  /** Resolves with the attachment's id once the server has reserved it. */
  reserved: Promise<string>;
  reserve: (attachmentId: string) => void;
  /** Resolves with the confirmed record; rejects when the upload is discarded. */
  settled: Promise<Attachment>;
  resolve: (attachment: Attachment) => void;
  reject: (error: unknown) => void;
  /** Changes asked for before the attachment was reserved, given to the server once it is. */
  changes: AttachmentChange | null;
}

/** Thrown to whoever waits for an upload that was discarded. */
export class UploadDiscardedError extends Error {
  constructor() {
    super("the upload was discarded");
    this.name = "UploadDiscardedError";
  }
}

/**
 * Why an upload failed, as its state says it: the server's words when it refused (a problem of
 * its own, not one made here for a request that never got an answer), and nothing otherwise,
 * which the app words itself.
 */
function failureOf(error: unknown): { cause: "refused" | "unreachable"; reason: string | null } {
  return error instanceof ApiProblemError && error.status >= 400 && error.code !== "internal"
    ? { cause: "refused", reason: error.message }
    : { cause: "unreachable", reason: null };
}

/**
 * The uploads of one session: begun with `add`, watched with `subscribe` and `get`, and gone
 * from here once they arrive (`settled` answers the record) or are discarded. One that fails
 * stays, to be sent again (`retry`) or discarded.
 */
export class AttachmentUploads {
  readonly #client: AspenClient;
  readonly #store: RecordStore;
  readonly #transport: UploadTransport;
  readonly #storage: PreferenceStorage | null;
  readonly #entries = new Map<string, Entry>();
  readonly #listeners = new Set<() => void>();
  #all: readonly Upload[] = [];
  #nextKey = 1;
  /** What `retain` was last told the server still waits for, for uploads found after it. */
  #waitedFor: ReadonlySet<string> | null = null;

  constructor(options: AttachmentUploadsOptions) {
    this.#client = options.client;
    this.#store = options.store;
    this.#transport = options.transport;
    this.#storage = options.storage ?? null;
  }

  /** Registers for any change to any upload; shaped for React's `useSyncExternalStore`. */
  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  /** Every upload under way or failed, in the order begun; the same array until one changes. */
  all = (): readonly Upload[] => this.#all;

  get(key: string): Upload | undefined {
    return this.#entries.get(key)?.upload;
  }

  /** The upload reserved as `attachmentId`, while it is under way or failed. */
  byAttachment(attachmentId: string): Upload | undefined {
    return this.#all.find((upload) => upload.attachmentId === attachmentId);
  }

  /** Whether any upload a sent message waits for is still on its way. */
  get sending(): boolean {
    for (const entry of this.#entries.values()) {
      if (entry.upload.detached && entry.upload.state.kind === "uploading") {
        return true;
      }
    }
    return false;
  }

  /**
   * Begins uploading `file`: reserves an attachment, sends the bytes, and confirms. `size` is
   * a picture's in pixels, which readers use to make room for it before it loads.
   */
  add(file: File, size?: { readonly width: number; readonly height: number }): Upload {
    const key = `u${String(this.#nextKey++)}`;
    const upload: Upload = {
      key,
      name: file.name,
      mimeType: file.type || "application/octet-stream",
      byteSize: file.size,
      attachmentId: null,
      file,
      detached: false,
      state: { kind: "uploading", sent: 0 },
    };
    this.#begin(upload, size, (entry) => this.#reserveAndSend(entry));
    return upload;
  }

  /** The attachment id of `key`, once reserved; rejects when reserving it failed. */
  reserved(key: string): Promise<string> {
    return this.#entries.get(key)?.reserved ?? Promise.reject(new UploadDiscardedError());
  }

  /**
   * The confirmed record of `key`, once it arrives, however many tries that takes; rejects with
   * `UploadDiscardedError` when it is discarded instead. A failure is read from its state.
   */
  settled(key: string): Promise<Attachment> {
    return this.#entries.get(key)?.settled ?? Promise.reject(new UploadDiscardedError());
  }

  /**
   * Changes what an upload under way shows, in its uploader's words, or whether it is a
   * spoiler: given to the server at once when the attachment is reserved, and as soon as it
   * is otherwise. Rejects when the server refuses the change.
   */
  async change(key: string, change: AttachmentChange): Promise<void> {
    const entry = this.#entries.get(key);
    if (entry === undefined) {
      return;
    }
    if (entry.upload.attachmentId === null) {
      entry.changes = { ...entry.changes, ...change };
      return;
    }
    await this.#patch(entry.upload.attachmentId, change);
  }

  /**
   * Sends a failed upload again: one never reserved is reserved now, and one that was is given
   * another URL for the same attachment, so a message already waiting for it needs no change.
   */
  retry(key: string): void {
    const entry = this.#entries.get(key);
    if (entry?.upload.state.kind !== "failed" || !entry.upload.state.retriable) {
      return;
    }
    const { upload, size, changes } = entry;
    this.#entries.delete(key);
    this.#begin(
      { ...upload, state: { kind: "uploading", sent: 0 } },
      size,
      (again) => {
        again.changes = changes;
        return upload.attachmentId === null
          ? this.#reserveAndSend(again)
          : this.#renewAndSend(again, upload.attachmentId);
      },
      // Whoever waited for the first try waits for this one.
      entry,
    );
  }

  /**
   * Marks the uploads `keys` as ones a sent message waits for: they go on with no message box
   * holding them, and are noted so that a page loaded after this one says an upload it finds
   * no longer under way was interrupted.
   */
  detach(keys: readonly string[]): void {
    this.#setDetached(keys, true);
  }

  /** Marks the uploads `keys` as a message box's again, a sent message waiting for them no more. */
  attach(keys: readonly string[]): void {
    this.#setDetached(keys, false);
  }

  #setDetached(keys: readonly string[], detached: boolean): void {
    for (const key of keys) {
      const entry = this.#entries.get(key);
      if (entry !== undefined) {
        entry.upload = { ...entry.upload, detached };
      }
    }
    this.#changed();
    this.#note();
  }

  /**
   * Stops and forgets an upload, and asks the server to delete what it reserved: for a file
   * taken out of the box, or a message deleted before its files arrived.
   */
  discard(key: string): void {
    const entry = this.#entries.get(key);
    if (entry === undefined) {
      return;
    }
    this.#drop(entry);
    entry.reject(new UploadDiscardedError());
    const id = entry.upload.attachmentId;
    if (id !== null) {
      this.#transport.discard?.(id);
      void this.#client.api
        .DELETE("/api/v1/attachments/{attachment}", { params: { path: { attachment: id } } })
        .catch(() => undefined);
    }
  }

  /** `discard` for each upload reserved as one of `attachmentIds`. */
  discardAttachments(attachmentIds: readonly string[]): void {
    for (const upload of this.#all) {
      if (upload.attachmentId !== null && attachmentIds.includes(upload.attachmentId)) {
        this.discard(upload.key);
      }
    }
  }

  /**
   * Finds what outlived the last page: uploads the transport kept going (a phone's, sent on
   * while the app was in the background or closed), which are watched to their end, and
   * uploads this storage noted that no transport kept, which a browser's page took with it and
   * are shown failed, with nothing to send again. Call once, when the session starts.
   */
  async restore(): Promise<void> {
    const kept = (await this.#transport.kept?.().catch(() => undefined)) ?? [];
    const follow = this.#transport.follow?.bind(this.#transport);
    const found = new Set<string>();
    for (const one of kept) {
      found.add(one.attachmentId);
      if (follow === undefined || this.byAttachment(one.attachmentId) !== undefined) {
        continue;
      }
      this.#begin(
        {
          key: `u${String(this.#nextKey++)}`,
          name: one.name,
          mimeType: one.mimeType,
          byteSize: one.byteSize,
          attachmentId: one.attachmentId,
          file: null,
          detached: true,
          state: { kind: "uploading", sent: 0 },
        },
        undefined,
        (entry) =>
          follow(
            one.attachmentId,
            (sent, total) => {
              this.#progress(entry, sent, total);
            },
            entry.abort.signal,
          ),
      );
    }
    const userId = this.#client.session?.userId;
    for (const noted of this.#noted()) {
      if (
        noted.userId !== userId ||
        found.has(noted.attachmentId) ||
        this.byAttachment(noted.attachmentId) !== undefined
      ) {
        continue;
      }
      const key = `u${String(this.#nextKey++)}`;
      const gone = Promise.reject(new UploadDiscardedError());
      gone.catch(() => undefined);
      this.#entries.set(key, {
        upload: {
          key,
          name: noted.name,
          mimeType: noted.mimeType,
          byteSize: noted.byteSize,
          attachmentId: noted.attachmentId,
          file: null,
          detached: true,
          state: { kind: "failed", cause: "interrupted", reason: null, retriable: false },
        },
        abort: new AbortController(),
        size: undefined,
        reserved: Promise.resolve(noted.attachmentId),
        reserve: () => undefined,
        settled: gone,
        resolve: () => undefined,
        reject: () => undefined,
        changes: null,
      });
    }
    this.#changed();
    // The server's answer came first: what it no longer waits for is let go now.
    if (this.#waitedFor !== null) {
      this.retain(this.#waitedFor);
    }
  }

  /**
   * Discards every upload a sent message waited for whose attachment is not among `waitedFor`,
   * the attachments of the messages the server still holds for their uploads: its message was
   * posted, cancelled, or dropped while this app was not listening.
   */
  retain(waitedFor: ReadonlySet<string>): void {
    this.#waitedFor = waitedFor;
    for (const entry of Array.from(this.#entries.values())) {
      const id = entry.upload.attachmentId;
      if (entry.upload.detached && id !== null && !waitedFor.has(id)) {
        this.#drop(entry);
        entry.reject(new UploadDiscardedError());
        this.#transport.discard?.(id);
      }
    }
  }

  /** Stops every upload this page is sending, as the session ends; nothing is deleted. */
  stop(): void {
    for (const entry of Array.from(this.#entries.values())) {
      entry.abort.abort();
      entry.reject(new UploadDiscardedError());
    }
    this.#entries.clear();
    this.#changed();
  }

  #begin(
    upload: Upload,
    size: Entry["size"],
    run: (entry: Entry) => Promise<Attachment>,
    waiters?: Pick<Entry, "resolve" | "reject" | "settled">,
  ): void {
    let resolve: (attachment: Attachment) => void = () => undefined;
    let reject: (error: unknown) => void = () => undefined;
    const settled =
      waiters?.settled ??
      new Promise<Attachment>((res, rej) => {
        resolve = res;
        reject = rej;
      });
    // Nobody may be waiting; a failure is then read from the upload's state.
    settled.catch(() => undefined);
    let reserve: (id: string) => void = () => undefined;
    let refuse: (error: unknown) => void = () => undefined;
    const reserved =
      upload.attachmentId === null
        ? new Promise<string>((res, rej) => {
            reserve = res;
            refuse = rej;
          })
        : Promise.resolve(upload.attachmentId);
    reserved.catch(() => undefined);
    const entry: Entry = {
      upload,
      abort: new AbortController(),
      size,
      reserved,
      reserve,
      settled,
      resolve: waiters?.resolve ?? resolve,
      reject: waiters?.reject ?? reject,
      changes: null,
    };
    this.#entries.set(upload.key, entry);
    this.#changed();
    run(entry).then(
      (attachment) => {
        if (this.#entries.get(upload.key) !== entry) {
          return;
        }
        this.#store.ingest({ attachments: [attachment] });
        this.#drop(entry);
        entry.resolve(attachment);
      },
      (error: unknown) => {
        refuse(error);
        if (this.#entries.get(upload.key) !== entry) {
          return;
        }
        this.#set(entry, {
          kind: "failed",
          ...failureOf(error),
          // A kept upload's bytes are the transport's, which sends them again itself.
          retriable: entry.upload.file !== null || this.#transport.kept !== undefined,
        });
      },
    );
  }

  async #reserveAndSend(entry: Entry): Promise<Attachment> {
    const { upload, size } = entry;
    const init = await this.#client.api.POST("/api/v1/attachments", {
      body: {
        fileName: upload.name,
        mimeType: upload.mimeType,
        byteSize: upload.byteSize,
        ...(size === undefined ? {} : { width: size.width, height: size.height }),
      },
      signal: entry.abort.signal,
    });
    if (init.data === undefined) {
      throw new ApiProblemError(problemOf(init.error, init.response));
    }
    entry.upload = { ...entry.upload, attachmentId: init.data.id };
    entry.reserve(init.data.id);
    this.#changed();
    if (entry.changes !== null) {
      const changes = entry.changes;
      entry.changes = null;
      // A change that fails to reach the server now is given again as the message is sent.
      await this.#patch(init.data.id, changes).catch(() => undefined);
    }
    return this.#send(entry, init.data);
  }

  async #renewAndSend(entry: Entry, attachmentId: string): Promise<Attachment> {
    const renewed = await this.#client.api.POST("/api/v1/attachments/{attachment}/renew", {
      params: { path: { attachment: attachmentId } },
      body: { byteSize: entry.upload.byteSize },
      signal: entry.abort.signal,
    });
    if (renewed.data === undefined) {
      throw new ApiProblemError(problemOf(renewed.error, renewed.response));
    }
    return this.#send(entry, renewed.data);
  }

  #send(
    entry: Entry,
    handle: components["schemas"]["AttachmentUploadHandle"],
  ): Promise<Attachment> {
    const { upload } = entry;
    this.#note();
    return this.#transport.send({
      attachmentId: handle.id,
      uploadUrl: requireSecure(handle.uploadUrl),
      // The type the URL is signed for: a deployment that does not say signs for the file's own.
      contentType: handle.contentType ?? upload.mimeType,
      name: upload.name,
      mimeType: upload.mimeType,
      byteSize: upload.byteSize,
      file: upload.file,
      onProgress: (sent, total) => {
        this.#progress(entry, sent, total);
      },
      signal: entry.abort.signal,
    });
  }

  async #patch(attachmentId: string, change: AttachmentChange): Promise<void> {
    // The answer is not kept: the store holds an attachment's record once it is confirmed, and
    // by its absence tells an upload still on its way.
    const { error, response } = await this.#client.api.PATCH("/api/v1/attachments/{attachment}", {
      params: { path: { attachment: attachmentId } },
      body: change,
    });
    if (error !== undefined) {
      throw new ApiProblemError(problemOf(error, response));
    }
  }

  #progress(entry: Entry, sent: number, total: number): void {
    if (entry.upload.state.kind === "uploading") {
      this.#set(entry, { kind: "uploading", sent: total > 0 ? Math.min(1, sent / total) : 0 });
    }
  }

  #set(entry: Entry, state: UploadState): void {
    entry.upload = { ...entry.upload, state };
    this.#changed();
  }

  #drop(entry: Entry): void {
    entry.abort.abort();
    this.#entries.delete(entry.upload.key);
    this.#changed();
    this.#note();
  }

  #changed(): void {
    this.#all = Array.from(this.#entries.values(), (entry) => entry.upload);
    for (const listener of this.#listeners) {
      listener();
    }
  }

  #noted(): readonly Noted[] {
    try {
      const kept: unknown = JSON.parse(this.#storage?.getItem(STORAGE_KEY) ?? "[]");
      return Array.isArray(kept) ? (kept as Noted[]) : [];
    } catch {
      return [];
    }
  }

  /** Writes down the uploads a sent message waits for, in place of those noted for this account. */
  #note(): void {
    if (this.#storage === null) {
      return;
    }
    const userId = this.#client.session?.userId;
    if (userId === undefined) {
      return;
    }
    const mine: Noted[] = [];
    for (const { upload } of this.#entries.values()) {
      if (upload.detached && upload.attachmentId !== null) {
        mine.push({
          userId,
          attachmentId: upload.attachmentId,
          name: upload.name,
          mimeType: upload.mimeType,
          byteSize: upload.byteSize,
        });
      }
    }
    const others = this.#noted().filter((noted) => noted.userId !== userId);
    try {
      if (mine.length + others.length === 0) {
        this.#storage.removeItem(STORAGE_KEY);
      } else {
        this.#storage.setItem(STORAGE_KEY, JSON.stringify([...others, ...mine]));
      }
    } catch {
      // Storage that is full or closed to the page notes nothing; the uploads go on.
    }
  }
}

/**
 * A browser's transport: `PUT`s the bytes to storage with `XMLHttpRequest`, which, unlike
 * `fetch`, says how much of a request body has been sent, then confirms with the server. The
 * request goes to the object store, not the API, and carries no session token.
 */
export function browserUploadTransport(
  client: AspenClient,
  /** Sends the bytes in place of `XMLHttpRequest`, where there is none; it reports no progress. */
  uploadFetch?: typeof globalThis.fetch,
): UploadTransport {
  return {
    async send(job) {
      if (job.file === null) {
        throw new ApiProblemError(transportProblem("the file is no longer held", 0));
      }
      const put =
        uploadFetch === undefined && typeof XMLHttpRequest !== "undefined"
          ? await putWithProgress(job.uploadUrl, job.file, job.contentType, job)
          : await (uploadFetch ?? globalThis.fetch)(job.uploadUrl, {
              method: "PUT",
              headers: { "content-type": job.contentType },
              body: job.file,
              signal: job.signal,
            }).then((response) => ({ status: response.status, statusText: response.statusText }));
      if (put.status < 200 || put.status >= 300) {
        throw new ApiProblemError(
          transportProblem(`upload failed: ${String(put.status)} ${put.statusText}`, put.status),
        );
      }
      const confirm = await client.api.POST("/api/v1/attachments/{attachment}/confirm", {
        params: { path: { attachment: job.attachmentId } },
      });
      if (confirm.data === undefined) {
        throw new ApiProblemError(problemOf(confirm.error, confirm.response));
      }
      return confirm.data;
    },
  };
}

function putWithProgress(
  url: string,
  body: Blob,
  contentType: string,
  job: Pick<UploadJob, "onProgress" | "signal">,
): Promise<{ status: number; statusText: string }> {
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open("PUT", url);
    request.setRequestHeader("content-type", contentType);
    request.upload.onprogress = (event) => {
      job.onProgress(event.loaded, event.lengthComputable ? event.total : body.size);
    };
    request.onload = () => {
      resolve({ status: request.status, statusText: request.statusText });
    };
    request.onerror = () => {
      reject(new TypeError("the upload could not reach storage"));
    };
    request.onabort = () => {
      reject(new UploadDiscardedError());
    };
    job.signal.addEventListener("abort", () => {
      request.abort();
    });
    if (job.signal.aborted) {
      reject(new UploadDiscardedError());
      return;
    }
    request.send(body);
  });
}
