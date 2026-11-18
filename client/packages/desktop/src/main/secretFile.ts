import { randomUUID } from "node:crypto";
import { readFile, rename, rm, writeFile } from "node:fs/promises";

/** The system's encryption, as `safeStorage` gives it. */
export interface SecretCipher {
  /** Whether a real system store backs it; never true for a fixed or absent key. */
  available(): Promise<boolean>;
  encrypt(text: string): Promise<Buffer>;
  /** `stale`: the system would now encrypt it otherwise, so it should be written again. */
  decrypt(data: Buffer): Promise<{ text: string; stale: boolean }>;
}

export type SecretRead =
  { status: "ok"; contents: string | null } | { status: "unavailable" } | { status: "unreadable" };

/**
 * Whether Chromium's Linux encryption used a key from a keyring (`v11`), rather than the fixed key
 * it falls back to (`v10`) without saying so.
 */
export function isKeyringCiphertext(data: Buffer): boolean {
  return data.subarray(0, 3).toString("latin1") === "v11";
}

function missing(error: unknown): boolean {
  return error instanceof Error && "code" in error && error.code === "ENOENT";
}

/**
 * One text kept encrypted in a file only its user may read. A write replaces the file by
 * renaming, so a crash leaves the old contents or the new, and writes run one at a time.
 * Nothing is ever written without the system's encryption.
 */
export class SecretFile {
  readonly #path: string;
  readonly #cipher: SecretCipher;
  #queue: Promise<unknown> = Promise.resolve();

  constructor(path: string, cipher: SecretCipher) {
    this.#path = path;
    this.#cipher = cipher;
  }

  async read(): Promise<SecretRead> {
    if (!(await this.#cipher.available())) {
      return { status: "unavailable" };
    }
    let data: Buffer;
    try {
      data = await readFile(this.#path);
    } catch (error) {
      if (missing(error)) {
        return { status: "ok", contents: null };
      }
      console.error("secrets: the file could not be read", error);
      return { status: "unreadable" };
    }
    try {
      const { text, stale } = await this.#cipher.decrypt(data);
      if (stale) {
        await this.write(text);
      }
      return { status: "ok", contents: text };
    } catch (error) {
      console.error("secrets: the system did not open the file", error);
      return { status: "unreadable" };
    }
  }

  write(contents: string): Promise<boolean> {
    const written = this.#queue.then(() => this.#replace(contents));
    this.#queue = written;
    return written;
  }

  async #replace(contents: string): Promise<boolean> {
    const temporary = `${this.#path}.${randomUUID()}`;
    try {
      if (!(await this.#cipher.available())) {
        return false;
      }
      const data = await this.#cipher.encrypt(contents);
      await writeFile(temporary, data, { mode: 0o600 });
      await rename(temporary, this.#path);
      return true;
    } catch (error) {
      console.error("secrets: the file could not be written", error);
      await rm(temporary, { force: true }).catch(() => undefined);
      return false;
    }
  }
}
