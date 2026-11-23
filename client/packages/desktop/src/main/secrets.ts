import { app, ipcMain, safeStorage } from "electron";
import { join } from "node:path";
import { isAppPage } from "./navigation";
import { SecretFile, isKeyringCiphertext, type SecretCipher } from "./secretFile";

/**
 * The app's saved sign-ins (`SessionVault`), encrypted with the key the system keeps for it: the
 * Keychain on macOS, DPAPI on Windows, and the Secret Service or KWallet on Linux, where opening
 * a locked keyring may ask for its password once as the app starts. On Linux without a running
 * keyring, `safeStorage` still reports a store and encrypts with a key every copy of Chromium
 * knows, so what it produced is what is checked (`isKeyringCiphertext`).
 */
const onLinux = process.platform === "linux";

async function encrypted(text: string): Promise<Buffer> {
  const data = await safeStorage.encryptStringAsync(text);
  if (onLinux && !isKeyringCiphertext(data)) {
    throw new Error("no keyring holds the encryption key");
  }
  return data;
}

const systemCipher: SecretCipher = {
  available: async () => {
    if (!(await safeStorage.isAsyncEncryptionAvailable())) {
      return false;
    }
    return !onLinux || isKeyringCiphertext(await safeStorage.encryptStringAsync("probe"));
  },
  encrypt: encrypted,
  decrypt: async (data) => {
    const { result, shouldReEncrypt } = await safeStorage.decryptStringAsync(data);
    return { text: result, stale: shouldReEncrypt };
  },
};

/**
 * For the drives that run an unpackaged shell where no keyring exists. It protects nothing, so a
 * packaged app never uses it.
 */
const drivesCipher: SecretCipher = {
  available: () => Promise.resolve(true),
  encrypt: (text) => Promise.resolve(Buffer.from(text)),
  decrypt: (data) => Promise.resolve({ text: data.toString(), stale: false }),
};

/** Answers the app's own page alone: other frames may hold a plugin's view. */
export function serveSecrets(page: Parameters<typeof isAppPage>[1]): void {
  const cipher =
    !app.isPackaged && process.env.ASPEN_DESKTOP_UNENCRYPTED_SESSIONS === "1"
      ? drivesCipher
      : systemCipher;
  const file = new SecretFile(join(app.getPath("userData"), "sessions.enc"), cipher);
  const fromApp = (event: Electron.IpcMainInvokeEvent) =>
    event.senderFrame !== null &&
    event.senderFrame.parent === null &&
    isAppPage(event.senderFrame.url, page);
  ipcMain.handle("secrets:read", (event) =>
    fromApp(event) ? file.read() : { status: "unavailable" },
  );
  ipcMain.handle("secrets:write", (event, contents: unknown) =>
    fromApp(event) && typeof contents === "string" ? file.write(contents) : false,
  );
}
