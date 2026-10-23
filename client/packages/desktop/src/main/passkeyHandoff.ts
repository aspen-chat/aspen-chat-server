import { BrowserWindow, app, ipcMain, shell } from "electron";
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
 * with a PKCE secret that never leaves it, along with the return code the browser brought here.
 */

export const PASSKEY_PREPARE_CHANNEL = "passkey:prepare";
export const PASSKEY_OPEN_CHANNEL = "passkey:open";
export const PASSKEY_DISPOSE_CHANNEL = "passkey:dispose";
/** Opens an identity provider's sign-in on a prepared handoff (`openProvider`). */
export const PASSKEY_OPEN_PROVIDER_CHANNEL = "passkey:open-provider";

/** How long the browser has to come back before the handoff counts as abandoned. */
const HANDOFF_TIMEOUT_MS = 10 * 60 * 1000;
const RETURN_PATH = "/passkey";

interface HandoffReturn {
  ceremony: string;
  outcome: "done" | "cancelled";
  code: string | null;
}

interface Pending {
  server: Server;
  returned: Promise<HandoffReturn>;
  /** Where the tab is sent once it has returned: the handoff page, told the outcome. */
  pageOrigin: string | null;
}

const pending = new Map<string, Pending>();

function parseReturn(url: URL): HandoffReturn | null {
  // An identity provider's flow comes back with its id and a return code alone
  // (`parseHandoffReturn` in `@aspen/protocol` reads it the same way).
  const flow = url.searchParams.get("flow");
  const flowCode = url.searchParams.get("code");
  if (
    url.pathname === RETURN_PATH &&
    flow !== null &&
    flowCode !== null &&
    !url.searchParams.has("ceremony")
  ) {
    return { ceremony: flow, outcome: "done", code: flowCode };
  }
  const ceremony = url.searchParams.get("ceremony");
  const outcome = url.searchParams.get("outcome");
  if (url.pathname !== RETURN_PATH || ceremony === null) {
    return null;
  }
  return outcome === "done" || outcome === "cancelled"
    ? { ceremony, outcome, code: url.searchParams.get("code") }
    : null;
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
    settle({ ceremony: "", outcome: "cancelled", code: null });
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
  // Only the server's own page is ever opened, never something the page could smuggle in, and
  // over HTTPS, as the renderer reaches the server (`securelyReached` in `@aspen/protocol`).
  if (!securePage(page) || page.pathname !== "/auth/passkey") {
    throw new Error("refusing to open a handoff page that is not /auth/passkey over HTTPS");
  }
  entry.pageOrigin = page.origin;
  await shell.openExternal(page.toString());
  return entry.returned;
}

/**
 * Opens an identity provider's sign-in (`authorizeUrl`) for a handoff prepared like a passkey
 * ceremony's: the provider returns the browser to the server, which sends it on to this
 * handoff's listener, and the tab is then sent to `server`'s passkey page to say it is done.
 */
async function openProvider(
  id: string,
  authorizeUrl: string,
  server: string,
): Promise<HandoffReturn> {
  const entry = pending.get(id);
  if (entry === undefined) {
    throw new Error("unknown passkey handoff");
  }
  const provider = new URL(authorizeUrl);
  const home = new URL(server);
  if (!securePage(provider) || !securePage(home)) {
    throw new Error("refusing to open an identity provider or a server not reached over HTTPS");
  }
  entry.pageOrigin = home.origin;
  await shell.openExternal(provider.toString());
  return entry.returned;
}

/**
 * Whether the page is served over HTTPS, or over plain HTTP at this machine's own addresses in
 * an unpackaged build, whose development server and deployment run there.
 */
function securePage(page: URL): boolean {
  if (page.protocol === "https:") {
    return true;
  }
  const loopback =
    page.hostname === "localhost" ||
    page.hostname.endsWith(".localhost") ||
    /^127\.\d+\.\d+\.\d+$/.test(page.hostname) ||
    page.hostname === "[::1]";
  return page.protocol === "http:" && loopback && !app.isPackaged;
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
  ipcMain.handle(
    PASSKEY_OPEN_PROVIDER_CHANNEL,
    (_event, id: unknown, authorizeUrl: unknown, server: unknown) => {
      if (
        typeof id !== "string" ||
        typeof authorizeUrl !== "string" ||
        typeof server !== "string"
      ) {
        throw new Error("passkey:open-provider takes a handoff id, a URL, and a server");
      }
      return openProvider(id, authorizeUrl, server);
    },
  );
  ipcMain.on(PASSKEY_DISPOSE_CHANNEL, (_event, id: unknown) => {
    if (typeof id === "string") {
      dispose(id);
    }
  });
}
