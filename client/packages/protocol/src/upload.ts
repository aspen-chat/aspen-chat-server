/**
 * Icons uploaded in the server's two phases (reserve, send the bytes straight to storage,
 * confirm), and changes to attachments not yet sent. Attachments themselves are uploaded by
 * `AttachmentUploads` (`uploads.ts`).
 */

import type { components } from "./generated/openapi";
import { type AspenClient, problemOf } from "./http";
import { ApiProblemError, transportProblem } from "./problem";
import { requireSecure } from "./urls";
import type { RecordStore } from "./store";

type Attachment = components["schemas"]["Attachment"];
type Icon = components["schemas"]["Icon"];

/** Where an upload goes and what it is cached in. */
export interface UploadTarget {
  readonly client: AspenClient;
  readonly store: RecordStore;
  readonly uploadFetch: typeof globalThis.fetch;
}

/** The longest description an attachment may have, in characters, as the server bounds it. */
export const ATTACHMENT_DESCRIPTION_MAX_CHARS = 1500;

/**
 * A change to an attachment not yet sent, as a merge patch: what it shows, in its uploader's
 * words (`null`, or nothing but space, clears it), and whether readers' apps cover it as a
 * spoiler until revealed.
 */
export type AttachmentChange = components["schemas"]["AttachmentUpdateRequest"];

/** Changes an attachment not yet sent, and caches the record as it now is. */
export async function updateAttachment(
  target: UploadTarget,
  attachmentId: string,
  change: AttachmentChange,
): Promise<Attachment> {
  const { data, error, response } = await target.client.api.PATCH(
    "/api/v1/attachments/{attachment}",
    {
      params: { path: { attachment: attachmentId } },
      body: change,
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
  const init = await target.client.api.POST("/api/v1/icons", {
    body: { mimeType, byteSize: bytes.size },
  });
  if (init.data === undefined) {
    throw new ApiProblemError(problemOf(init.error, init.response));
  }
  const put = await target.uploadFetch(requireSecure(init.data.uploadUrl), {
    method: "PUT",
    headers: { "content-type": init.data.contentType ?? mimeType },
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
