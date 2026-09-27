import { BrowserWindow, ipcMain, shell } from "electron";
import { randomUUID } from "node:crypto";
import { createServer, type Server } from "node:http";

/**
 * Passkey ceremonies in the system browser. A passkey belongs to the server's domain, and the
 * app's page is not on it, so the renderer hands each ceremony to the server's `/auth/passkey`
 * page in the user's browser, where their passkey managers and phone are. The browser comes
 * back to a one-shot listener on the loopback interface (RFC 8252 §7.3), which answers by
 * sending the tab back to the server's page to say it is done, and tells the renderer.
 *
 * Any local process can reach the listener, which is harmless: the renderer claims the result
 * with a PKCE secret that never leaves it.
 */

export const PASSKEY_PREPARE_CHANNEL = "passkey:prepare";
export const PASSKEY_OPEN_CHANNEL = "passkey:open";
export const PASSKEY_DISPOSE_CHANNEL = "passkey:dispose";

/** How long the browser has to come back before the handoff counts as abandoned. */
const HANDOFF_TIMEOUT_MS = 10 * 60 * 1000;
const RETURN_PATH = "/passkey";

interface HandoffReturn {
  ceremony: string;
  outcome: "done" | "cancelled";
}

interface Pending {
  server: Server;
  returned: Promise<HandoffReturn>;
  /** Where the tab is sent once it has returned: the handoff page, told the outcome. */
  pageOrigin: string | null;
}

const pending = new Map<string, Pending>();

function parseReturn(url: URL): HandoffReturn | null {
  const ceremony = url.searchParams.get("ceremony");
  const outcome = url.searchParams.get("outcome");
  if (url.pathname !== RETURN_PATH || ceremony === null) {
    return null;
  }
  return outcome === "done" || outcome === "cancelled" ? { ceremony, outcome } : null;
}

async function prepare(): Promise<{ id: string; returnTo: string }> {
  const id = randomUUID();
  let settle: (value: HandoffReturn) => void = () => undefined;
  const returned = new Promise<HandoffReturn>((resolve) => {
    settle = resolve;
  });
  const entry: Pending = { server: createServer(), returned, pageOrigin: null };
  entry.server.on("request", (request, response) => {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    const result = parseReturn(url);
    if (result === null) {
      response.writeHead(404).end();
      return;
    }
    // The page shows a closing message in the browser's own language.
    const next =
      entry.pageOrigin === null
        ? null
        : `${entry.pageOrigin}/auth/passkey#finished=${result.outcome}`;
    response.writeHead(next === null ? 204 : 302, next === null ? {} : { Location: next }).end();
    settle(result);
    const window = BrowserWindow.getAllWindows()[0];
    if (window !== undefined) {
      if (window.isMinimized()) {
        window.restore();
      }
      window.focus();
    }
    dispose(id);
  });
  await new Promise<void>((resolve, reject) => {
    entry.server.once("error", reject);
    entry.server.listen(0, "127.0.0.1", () => {
      resolve();
    });
  });
  const address = entry.server.address();
  if (address === null || typeof address === "string") {
    entry.server.close();
    throw new Error("the loopback listener has no port");
  }
  pending.set(id, entry);
  setTimeout(() => {
    settle({ ceremony: "", outcome: "cancelled" });
    dispose(id);
  }, HANDOFF_TIMEOUT_MS).unref();
  return { id, returnTo: `http://127.0.0.1:${String(address.port)}${RETURN_PATH}` };
}

async function open(id: string, url: string): Promise<HandoffReturn> {
  const entry = pending.get(id);
  if (entry === undefined) {
    throw new Error("unknown passkey handoff");
  }
  const page = new URL(url);
  // Only the server's own page is ever opened, never something the page could smuggle in.
  if (!["http:", "https:"].includes(page.protocol) || page.pathname !== "/auth/passkey") {
    throw new Error("refusing to open a handoff page that is not /auth/passkey");
  }
  entry.pageOrigin = page.origin;
  await shell.openExternal(page.toString());
  return entry.returned;
}

function dispose(id: string): void {
  const entry = pending.get(id);
  if (entry !== undefined) {
    pending.delete(id);
    entry.server.close();
  }
}

export function servePasskeyHandoff(): void {
  ipcMain.handle(PASSKEY_PREPARE_CHANNEL, () => prepare());
  ipcMain.handle(PASSKEY_OPEN_CHANNEL, (_event, id: unknown, url: unknown) => {
    if (typeof id !== "string" || typeof url !== "string") {
      throw new Error("passkey:open takes a handoff id and a URL");
    }
    return open(id, url);
  });
  ipcMain.on(PASSKEY_DISPOSE_CHANNEL, (_event, id: unknown) => {
    if (typeof id === "string") {
      dispose(id);
    }
  });
}
