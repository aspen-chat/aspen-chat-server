import { saveFile } from "@/features/layout/saveFile";

/**
 * A file the deployment's storage would not let the app read: its read path sends no
 * `Access-Control-Allow-Origin` (docs/operators/installing/storage-read-path.md), or could not
 * be reached.
 */
export class UnreadableFile extends Error {}

/**
 * Reads an attachment, or a link's picture, from the deployment's storage, which lets any origin
 * read what it serves, without credentials.
 */
async function readFile(url: string): Promise<Blob> {
  let response: Response;
  try {
    response = await fetch(url, { mode: "cors", credentials: "omit" });
  } catch {
    throw new UnreadableFile();
  }
  if (!response.ok) {
    throw new Error(`${String(response.status)} ${response.statusText}`);
  }
  return response.blob();
}

/** Saves the file at `url` under `name`, as `saveFile` saves any. */
export async function saveFrom(url: string, name: string): Promise<boolean> {
  return saveFile(await readFile(url), name);
}

/** `blob`, a picture, as a PNG: the one picture type every clipboard takes. */
async function asPng(blob: Blob): Promise<Blob> {
  if (blob.type === "image/png") {
    return blob;
  }
  const bitmap = await createImageBitmap(blob);
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  canvas.getContext("2d")?.drawImage(bitmap, 0, 0);
  bitmap.close();
  return new Promise((resolve, reject) => {
    canvas.toBlob((png) => {
      if (png === null) {
        reject(new Error("the picture could not be drawn"));
      } else {
        resolve(png);
      }
    }, "image/png");
  });
}

/** Whether pictures can be put on the clipboard here: on a secure page with the Clipboard API. */
export function canCopyPictures(): boolean {
  return (
    window.isSecureContext &&
    typeof ClipboardItem !== "undefined" &&
    typeof navigator.clipboard.write === "function"
  );
}

/**
 * Puts the picture at `url` on the clipboard, as a PNG. The item is made at once, with a promise
 * of the picture, since Safari takes a clipboard write only in the press that asked for it.
 */
export async function copyPicture(url: string): Promise<void> {
  let failure: unknown = null;
  const png = readFile(url)
    .then(asPng)
    .catch((e: unknown) => {
      failure = e;
      throw e;
    });
  try {
    await navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
  } catch (e) {
    throw failure ?? e;
  }
}
