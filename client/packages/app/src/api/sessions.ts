import {
  SESSION_KEY,
  SessionVault,
  WebStorageSessionStore,
  dropLegacySessions,
  type SessionStore,
  type VaultBackend,
  type VaultRead,
} from "@aspen/protocol";
import { registerPlugin } from "@capacitor/core";
import { detectShell, type Shell } from "@/config";

/**
 * Where this shell keeps sign-ins. A browser tab has no key of the system's, so its sessions stay
 * in web storage. The desktop and mobile apps keep theirs in a `SessionVault`, opened once before
 * any client is made and never written anywhere unencrypted.
 */

interface AspenSecretsPlugin {
  read(): Promise<{ status: VaultRead["status"]; contents?: string | null }>;
  write(options: { contents: string }): Promise<{ kept: boolean }>;
}

const AspenSecrets = registerPlugin<AspenSecretsPlugin>("AspenSecrets");

const phoneBackend: VaultBackend = {
  read: async () => {
    const { status, contents } = await AspenSecrets.read();
    return status === "ok" ? { status, contents: contents ?? null } : { status };
  },
  write: async (contents) => (await AspenSecrets.write({ contents })).kept,
};

function backendOf(shell: Shell): VaultBackend | null {
  switch (shell) {
    case "web":
      return null;
    case "desktop":
      return window.aspenDesktop?.secrets ?? null;
    case "mobile":
      return phoneBackend;
  }
}

export type SavedSignIns = "open" | "unavailable" | "unreadable";

let vault: SessionVault | null = null;
let opening: Promise<SavedSignIns> | null = null;

function storage(): Storage | undefined {
  try {
    return window.localStorage;
  } catch {
    return undefined;
  }
}

/** Whether sessions can be read at once, as they always can in a browser tab. */
export function savedSignInsOpen(): boolean {
  return vault !== null || backendOf(detectShell()) === null;
}

async function open(backend: VaultBackend): Promise<SavedSignIns> {
  try {
    const opened = await SessionVault.open(backend, storage());
    if (opened.status !== "open") {
      return opened.status;
    }
    vault = opened.vault;
    return "open";
  } catch (error) {
    console.error("sessions: the system's store did not answer", error);
    return "unavailable";
  }
}

/** Opens the vault, once however often it is asked, or again after a failure. */
export function openSavedSignIns(): Promise<SavedSignIns> {
  const backend = backendOf(detectShell());
  if (backend === null || vault !== null) {
    return Promise.resolve("open");
  }
  opening ??= open(backend).finally(() => {
    opening = null;
  });
  return opening;
}

function forgetUnencrypted(): void {
  const legacy = storage();
  if (legacy !== undefined) {
    dropLegacySessions(legacy);
  }
}

/** Goes on with sessions that last until the app closes. */
export function continueWithoutSaving(): void {
  forgetUnencrypted();
  vault = SessionVault.inMemory();
}

/** Forgets the sign-ins kept here, which cannot be opened, and keeps new ones in their place. */
export async function startSavedSignInsOver(): Promise<void> {
  const backend = backendOf(detectShell());
  if (backend === null) {
    return;
  }
  vault = await SessionVault.emptied(backend);
  forgetUnencrypted();
}

/** The store of the session kept under `key`. */
export function sessionStore(key: string = SESSION_KEY): SessionStore {
  return vault?.store(key) ?? new WebStorageSessionStore(window.localStorage, key);
}
