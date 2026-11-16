import type { FileSink, PickedFile } from "@aspen/protocol";
import { Capacitor, registerPlugin } from "@capacitor/core";

/**
 * The mobile apps' native side of the files a call sends and receives (`AspenFilesPlugin`, in
 * each). Receiving: the system's picker chooses where a file goes before it arrives, offering
 * the name it was sent with, and the file is written to it by id as it comes, by the page
 * (`write`, in base64, gathered into pieces of about `PIECE_BYTES` so that the bridge is crossed
 * rarely) or by the native call's engine, which is given the same id (`FileSink.destination`).
 * Offering: the system's picker chooses a file to offer (`pick`), which the native side holds by
 * a reference of its own for the engine to read, until `release`; the file's bytes never cross
 * the bridge.
 */
interface AspenFilesPlugin {
  create(options: { name: string; mimeType?: string }): Promise<{ id: string | null }>;
  write(options: { id: string; data: string }): Promise<void>;
  close(options: { id: string }): Promise<void>;
  abort(options: { id: string }): Promise<void>;
  pick(): Promise<{ reference: string; name: string; size: number } | { reference: null }>;
  release(options: { reference: string }): Promise<void>;
}

const AspenFiles = registerPlugin<AspenFilesPlugin>("AspenFiles");

/** How much is gathered before it is handed to the native side. */
const PIECE_BYTES = 1024 * 1024;

/** Whether this is a mobile app, whose native side can choose where a file goes. */
export function nativeCanChooseDestination(): boolean {
  return Capacitor.isNativePlatform() && Capacitor.isPluginAvailable("AspenFiles");
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

/**
 * Asks for a file to offer through the system's picker, which the native side then holds by
 * `reference` for a native call's engine to read; `null` when the user backed out. A picked file
 * not offered is let go with `nativeReleaseFile`; one offered is let go by the engine.
 */
export async function nativePickFile(): Promise<PickedFile | null> {
  const picked = await AspenFiles.pick();
  if (picked.reference === null) {
    return null;
  }
  return { reference: picked.reference, name: picked.name, size: picked.size };
}

/** Lets the native side let go of a picked file that was not offered. */
export function nativeReleaseFile(reference: string): Promise<void> {
  return AspenFiles.release({ reference });
}

/**
 * Asks where to save `name` through the system picker; `null` when the user backed out. The
 * sink names the place by `destination`, so a native call's engine can write to it instead.
 */
export async function nativeChooseDestination(name: string): Promise<FileSink | null> {
  const { id } = await AspenFiles.create({ name });
  if (id === null) {
    return null;
  }
  let held: ArrayBuffer[] = [];
  let heldBytes = 0;
  const flush = async (): Promise<void> => {
    if (heldBytes === 0) {
      return;
    }
    const piece = new Blob(held);
    held = [];
    heldBytes = 0;
    await AspenFiles.write({ id, data: await base64Of(piece) });
  };
  return {
    destination: id,
    async write(chunk) {
      held.push(chunk);
      heldBytes += chunk.byteLength;
      if (heldBytes >= PIECE_BYTES) {
        await flush();
      }
    },
    async close() {
      await flush();
      await AspenFiles.close({ id });
    },
    async abort() {
      held = [];
      heldBytes = 0;
      await AspenFiles.abort({ id });
    },
  };
}
