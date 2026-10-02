/**
 * A custom emoji's picture as the server takes it: a PNG, JPEG, WebP, or GIF of at most
 * `EMOJI_MAX_PX` on a side and `EMOJI_MAX_BYTES`. A still picture larger than that is scaled
 * down here, to a PNG; a GIF cannot be rescaled without losing its frames, so one too large is
 * refused with what to do.
 */

export const EMOJI_MAX_PX = 128;
export const EMOJI_MAX_BYTES = 256 * 1024;
export const EMOJI_TYPES: readonly string[] = [
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/gif",
];

/** Why a picture cannot be an emoji. */
export type PictureProblem = "type" | "size" | "gifTooLarge";

export type PreparedPicture = { blob: Blob; mimeType: string } | { problem: PictureProblem };

export async function prepareEmojiPicture(file: File): Promise<PreparedPicture> {
  if (!EMOJI_TYPES.includes(file.type)) {
    return { problem: "type" };
  }
  const bitmap = await createImageBitmap(file);
  try {
    const fits = bitmap.width <= EMOJI_MAX_PX && bitmap.height <= EMOJI_MAX_PX;
    if (file.type === "image/gif") {
      if (!fits) {
        return { problem: "gifTooLarge" };
      }
      return file.size <= EMOJI_MAX_BYTES
        ? { blob: file, mimeType: file.type }
        : { problem: "size" };
    }
    if (fits && file.size <= EMOJI_MAX_BYTES) {
      return { blob: file, mimeType: file.type };
    }
    const scale = Math.min(1, EMOJI_MAX_PX / Math.max(bitmap.width, bitmap.height));
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(bitmap.width * scale));
    canvas.height = Math.max(1, Math.round(bitmap.height * scale));
    const context = canvas.getContext("2d");
    if (context === null) {
      return { problem: "size" };
    }
    context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
    const blob = await new Promise<Blob | null>((resolve) => {
      canvas.toBlob(resolve, "image/png");
    });
    if (blob === null || blob.size > EMOJI_MAX_BYTES) {
      return { problem: "size" };
    }
    return { blob, mimeType: "image/png" };
  } finally {
    bitmap.close();
  }
}
