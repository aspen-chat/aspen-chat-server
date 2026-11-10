import {
  ApiProblemError,
  UploadDiscardedError,
  acceptLanguage,
  type AspenClient,
  type Attachment,
  type KeptUpload,
  type UploadTransport,
} from "@aspen/protocol";
import { Capacitor, registerPlugin, type PluginListenerHandle } from "@capacitor/core";
import type { Messages } from "@/i18n/messages";
import { format } from "@/i18n/messages";

/**
 * The mobile apps' native side of uploading attachments (`AspenUploadsPlugin`, in each). A
 * page's own requests stop when the app goes to the background, so the app's native code
 * sends each file instead: the page hands it the bytes, which it keeps in the app's storage,
 * and where they go, and it sends them whether or not the app is in front, then confirms the
 * upload with the deployment itself, as the signed-in user. A message sent before its files
 * had arrived (`AspenSync.uploads`) is so posted the moment the last arrives, with no page
 * awake to see it.
 *
 * The native side keeps what it is sending across the page's reloads and the app's restarts
 * (`list`), each upload under its attachment's id, and lets one go only when told (`discard`).
 */
interface AspenUploadsPlugin {
  /** Makes an empty place for the bytes of upload `id`, in place of any it held. */
  begin(options: { id: string }): Promise<void>;
  /** Adds `data`, base64, to the bytes of upload `id`. */
  write(options: { id: string; data: string }): Promise<void>;
  /**
   * Sends the bytes it holds for `id` to `uploadUrl`, as `contentType`, then confirms the
   * upload at `origin` as `userId` (`POST /api/v1/attachments/{id}/confirm`), refreshing the
   * session with `refreshToken` when `sessionToken` has expired, and asking for another URL
   * (`POST /api/v1/attachments/{id}/renew`) when storage says this one has. Resolves once the
   * upload is under way; `settled` says how it ends. Called again for an upload that failed,
   * with another URL, it sends the same bytes again.
   */
  send(options: {
    id: string;
    origin: string;
    userId: string;
    sessionToken: string;
    refreshToken: string;
    uploadUrl: string;
    contentType: string;
    name: string;
    mimeType: string;
    byteSize: number;
    /**
     * The languages to ask the deployment to answer in (`Accept-Language`), so a refusal reads
     * in the app's language rather than the phone's; empty to leave it to the system.
     */
    acceptLanguage: string;
    /** A notification's words, should the upload fail while the app is not in front. */
    failedTitle: string;
    failedBody: string;
    /** Android: the words of the notification that stands while files are sent. */
    sendingTitle: string;
  }): Promise<void>;
  /** Every upload it holds for `userId` at `origin`. */
  list(options: { origin: string; userId: string }): Promise<{ uploads: NativeUpload[] }>;
  /** Stops upload `id` if it is under way, and forgets it and its bytes. */
  discard(options: { id: string }): Promise<void>;
  addListener(
    event: "progress",
    listener: (progress: { id: string; sent: number; total: number }) => void,
  ): Promise<PluginListenerHandle>;
  addListener(
    event: "settled",
    listener: (upload: NativeUpload) => void,
  ): Promise<PluginListenerHandle>;
}

/** An upload as the native side holds it. */
interface NativeUpload {
  /** The attachment's id. */
  id: string;
  name: string;
  mimeType: string;
  byteSize: number;
  state: "sending" | "done" | "failed";
  /** How many bytes have reached storage. */
  sent: number;
  /** Once `done`: the confirmed attachment record, as the deployment answered, JSON text. */
  attachment?: string;
  /** Once `failed`: whether the deployment refused it, or it could not be sent. */
  cause?: "refused" | "unreachable";
  /** For a refusal, the deployment's own words. */
  reason?: string | null;
}

const AspenUploads = registerPlugin<AspenUploadsPlugin>("AspenUploads");

/** How much of a file crosses the bridge at a time. */
const PIECE_BYTES = 1024 * 1024;

/** Whether this is a mobile app whose native side sends uploads. */
export function nativeSendsUploads(): boolean {
  return Capacitor.isNativePlatform() && Capacitor.isPluginAvailable("AspenUploads");
}

/** The words the native side shows on its own, in the reader's language as it is now. */
let words: Pick<
  Messages["pending"],
  "notifyFailedTitle" | "notifyFailedBody" | "notifySending"
> | null = null;

/** Gives the native side's notifications the reader's language. */
export function wordNativeUploads(m: Messages): void {
  words = m.pending;
}

/** Base64 of `blob`, read by the browser rather than a loop over its bytes. */
function base64Of(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const url = typeof reader.result === "string" ? reader.result : "";
      resolve(url.slice(url.indexOf(",") + 1));
    };
    reader.onerror = () => {
      reject(reader.error ?? new Error("the file could not be read"));
    };
    reader.readAsDataURL(blob);
  });
}

/** Whoever waits for an upload to end, by attachment id, across every deployment's transport. */
const waiting = new Map<
  string,
  { progress: (sent: number, total: number) => void; settle: (upload: NativeUpload) => void }
>();
/** Each transport's look at how the uploads it holds stand, run when the app comes back. */
const rechecks = new Set<() => void>();
let listening = false;

/** How an upload that failed is thrown, so its cause reads as the uploads' own do. */
function failureOf(upload: NativeUpload): Error {
  return upload.cause === "refused" && upload.reason != null
    ? new ApiProblemError({
        code: "validation",
        title: upload.reason,
        detail: upload.reason,
        status: 400,
      })
    : new TypeError("the upload could not reach storage");
}

/**
 * Hears the native side's uploads, once for the page: progress and endings as they happen,
 * and, each time the app comes back to the front, how each upload waited for stands, since
 * what ended while the page slept may not have been heard.
 */
function listen(recheck: () => void): void {
  rechecks.add(recheck);
  if (listening) {
    return;
  }
  listening = true;
  void AspenUploads.addListener("progress", ({ id, sent, total }) => {
    waiting.get(id)?.progress(sent, total);
  });
  void AspenUploads.addListener("settled", (upload) => {
    waiting.get(upload.id)?.settle(upload);
  });
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") {
      for (const look of rechecks) {
        look();
      }
    }
  });
}

/**
 * A transport that has the app's native side send each upload, for `client`'s deployment and
 * account. Uploads it finds the native side still holding when the page loads are the
 * session's again (`kept`), followed to their end or sent again from the bytes kept there.
 */
export function nativeUploadTransport(client: AspenClient): UploadTransport {
  const account = () => {
    const session = client.session;
    if (session === null) {
      throw new Error("not signed in");
    }
    return { origin: client.baseUrl, ...session };
  };
  const held = async (): Promise<NativeUpload[]> => {
    const { origin, userId } = account();
    return (await AspenUploads.list({ origin, userId })).uploads;
  };
  const recheck = () => {
    void held()
      .then((uploads) => {
        for (const upload of uploads) {
          waiting.get(upload.id)?.settle(upload);
        }
      })
      .catch(() => undefined);
  };
  const follow = (
    attachmentId: string,
    onProgress: (sent: number, total: number) => void,
    signal: AbortSignal,
  ): Promise<Attachment> =>
    new Promise<Attachment>((resolve, reject) => {
      listen(recheck);
      const stop = () => {
        waiting.delete(attachmentId);
        signal.removeEventListener("abort", abandon);
      };
      const abandon = () => {
        stop();
        void AspenUploads.discard({ id: attachmentId }).catch(() => undefined);
        reject(new UploadDiscardedError());
      };
      signal.addEventListener("abort", abandon);
      waiting.set(attachmentId, {
        progress: onProgress,
        settle: (upload) => {
          if (upload.state === "done" && upload.attachment !== undefined) {
            stop();
            // The record is the session's now; the native side need keep nothing of it.
            void AspenUploads.discard({ id: attachmentId }).catch(() => undefined);
            resolve(JSON.parse(upload.attachment) as Attachment);
          } else if (upload.state === "failed") {
            stop();
            reject(failureOf(upload));
          } else {
            onProgress(upload.sent, upload.byteSize);
          }
        },
      });
      // What ended between the page asking and its listening.
      recheck();
    });
  return {
    async send(job) {
      const { origin, userId, sessionToken, refreshToken } = account();
      if (job.file !== null) {
        await AspenUploads.begin({ id: job.attachmentId });
        for (let at = 0; at < job.file.size; at += PIECE_BYTES) {
          if (job.signal.aborted) {
            void AspenUploads.discard({ id: job.attachmentId }).catch(() => undefined);
            throw new UploadDiscardedError();
          }
          await AspenUploads.write({
            id: job.attachmentId,
            data: await base64Of(job.file.slice(at, at + PIECE_BYTES)),
          });
        }
      }
      await AspenUploads.send({
        id: job.attachmentId,
        origin,
        userId,
        sessionToken,
        refreshToken,
        uploadUrl: job.uploadUrl,
        contentType: job.contentType,
        name: job.name,
        mimeType: job.mimeType,
        byteSize: job.byteSize,
        acceptLanguage: acceptLanguage() ?? "",
        failedTitle: words?.notifyFailedTitle ?? "",
        failedBody: format(words?.notifyFailedBody ?? "{name}", { name: job.name }),
        sendingTitle: words?.notifySending ?? "",
      });
      return follow(job.attachmentId, job.onProgress, job.signal);
    },
    async kept(): Promise<readonly KeptUpload[]> {
      const uploads = await held();
      const kept: KeptUpload[] = [];
      for (const upload of uploads) {
        const waiter = waiting.get(upload.id);
        if (waiter !== undefined) {
          // Sent by this page, which hears of it here if it has not already.
          waiter.settle(upload);
        } else if (upload.state === "done") {
          // It arrived with no page awake, and the deployment has since done what waited for
          // it; nothing here does.
          void AspenUploads.discard({ id: upload.id }).catch(() => undefined);
        } else {
          kept.push({
            attachmentId: upload.id,
            name: upload.name,
            mimeType: upload.mimeType,
            byteSize: upload.byteSize,
          });
        }
      }
      return kept;
    },
    follow,
    discard(attachmentId) {
      void AspenUploads.discard({ id: attachmentId }).catch(() => undefined);
    },
  };
}
