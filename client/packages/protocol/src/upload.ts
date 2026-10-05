/**
 * Uploads in the server's two phases: reserve, send the bytes straight to storage, confirm.
 */

import type { components } from "./generated/openapi";
import { type AspenClient, problemOf } from "./http";
import { ApiProblemError, transportProblem } from "./problem";
import type { RecordStore } from "./store";

type Attachment = components["schemas"]["Attachment"];
type Icon = components["schemas"]["Icon"];

/** Where an upload goes and what it is cached in. */
export interface UploadTarget {
  readonly client: AspenClient;
  readonly store: RecordStore;
  readonly uploadFetch: typeof globalThis.fetch;
  /** Whether uploads go through a `fetch` of the caller's rather than the browser's own. */
  readonly customUpload: boolean;
}

/**
 * Uploads a file in the server's two phases, reserving an attachment, sending the bytes
 * straight to storage, and confirming, and caches the resulting record. The attachment can
 * then be named in a message.
 */
export async function uploadAttachment(
  target: UploadTarget,
  file: File,
  /** A picture's size in pixels, which readers use to make room for it before it loads. */
  size?: { readonly width: number; readonly height: number },
  /** Told how many of the file's bytes have reached storage, as they go. */
  onProgress?: (sent: number, total: number) => void,
): Promise<Attachment> {
  const init = await target.client.api.POST("/api/v1/attachments", {
    body: {
      fileName: file.name,
      mimeType: file.type || "application/octet-stream",
      ...(size === undefined ? {} : { width: size.width, height: size.height }),
    },
  });
  if (init.data === undefined) {
    throw new ApiProblemError(problemOf(init.error, init.response));
  }
  const contentType = file.type || "application/octet-stream";
  const put =
    onProgress !== undefined && !target.customUpload && typeof XMLHttpRequest !== "undefined"
      ? await putWithProgress(init.data.uploadUrl, file, contentType, onProgress)
      : await target
          .uploadFetch(init.data.uploadUrl, {
            method: "PUT",
            headers: { "content-type": contentType },
            body: file,
          })
          .then((response) => ({ status: response.status, statusText: response.statusText }));
  if (put.status < 200 || put.status >= 300) {
    throw new ApiProblemError(
      transportProblem(`upload failed: ${String(put.status)} ${put.statusText}`, put.status),
    );
  }
  const confirm = await target.client.api.POST("/api/v1/attachments/{attachment}/confirm", {
    params: { path: { attachment: init.data.id } },
  });
  if (confirm.data === undefined) {
    throw new ApiProblemError(problemOf(confirm.error, confirm.response));
  }
  target.store.ingest({ attachments: [confirm.data] });
  return confirm.data;
}

/** The longest description an attachment may have, in characters, as the server bounds it. */
export const ATTACHMENT_DESCRIPTION_MAX_CHARS = 1500;

/**
 * Sets or clears (with `null`, or nothing but space) what an attachment not yet sent shows, in
 * its uploader's words, and caches the record as it now is.
 */
export async function describeAttachment(
  target: UploadTarget,
  attachmentId: string,
  description: string | null,
): Promise<Attachment> {
  const { data, error, response } = await target.client.api.PATCH(
    "/api/v1/attachments/{attachment}",
    {
      params: { path: { attachment: attachmentId } },
      body: { description },
    },
  );
  if (data === undefined) {
    throw new ApiProblemError(problemOf(error, response));
  }
  target.store.ingest({ attachments: [data] });
  return data;
}

/**
 * Uploads an icon in the server's two phases, reserving it, sending the bytes straight to
 * storage, and confirming, and caches the record. The caller then names the icon on a user
 * or community.
 */
export async function uploadIcon(
  target: UploadTarget,
  bytes: Blob,
  mimeType: string,
): Promise<Icon> {
  const init = await target.client.api.POST("/api/v1/icons", { body: { mimeType } });
  if (init.data === undefined) {
    throw new ApiProblemError(problemOf(init.error, init.response));
  }
  const put = await target.uploadFetch(init.data.uploadUrl, {
    method: "PUT",
    headers: { "content-type": mimeType },
    body: bytes,
  });
  if (!put.ok) {
    throw new ApiProblemError(
      transportProblem(`upload failed: ${String(put.status)} ${put.statusText}`, put.status),
    );
  }
  const confirm = await target.client.api.POST("/api/v1/icons/{icon}/confirm", {
    params: { path: { icon: init.data.id } },
  });
  if (confirm.data === undefined) {
    throw new ApiProblemError(problemOf(confirm.error, confirm.response));
  }
  target.store.putIcon(confirm.data);
  return confirm.data;
}

/**
 * `PUT`s `body` to storage with `XMLHttpRequest`, which, unlike `fetch`, says how much of a
 * request body has been sent. A failure to reach storage at all rejects, as `fetch`'s would.
 */
function putWithProgress(
  url: string,
  body: Blob,
  contentType: string,
  onProgress: (sent: number, total: number) => void,
): Promise<{ status: number; statusText: string }> {
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open("PUT", url);
    request.setRequestHeader("content-type", contentType);
    request.upload.onprogress = (event) => {
      onProgress(event.loaded, event.lengthComputable ? event.total : body.size);
    };
    request.onload = () => {
      resolve({ status: request.status, statusText: request.statusText });
    };
    request.onerror = () => {
      reject(new TypeError("the upload could not reach storage"));
    };
    request.onabort = request.onerror;
    request.send(body);
  });
}
