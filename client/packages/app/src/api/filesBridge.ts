import type { FileSink } from "@aspen/protocol";
import { Capacitor, registerPlugin } from "@capacitor/core";

/**
 * The mobile apps' native side of receiving files (`AspenFilesPlugin`, in each): the system's
 * picker chooses where a file goes before it arrives (on Android the file itself, on iOS the
 * folder it is made in, under the name it was sent with), and the transfer writes to it by id
 * as it comes. Writes cross the bridge as base64, gathered into pieces of about
 * `PIECE_BYTES` so that the bridge is crossed rarely.
 */
interface AspenFilesPlugin {
  create(options: { name: string; mimeType?: string }): Promise<{ id: string | null }>;
  write(options: { id: string; data: string }): Promise<void>;
  close(options: { id: string }): Promise<void>;
  abort(options: { id: string }): Promise<void>;
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

/** Asks where to save `name` through the system picker; `null` when the user backed out. */
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
