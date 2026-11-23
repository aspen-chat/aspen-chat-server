import assert from "node:assert/strict";
import { mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, describe, it } from "node:test";
import { SecretFile, isKeyringCiphertext, type SecretCipher } from "./secretFile.ts";

const reversed = (text: string) => Array.from(text).reverse().join("");

function reversing(overrides: Partial<SecretCipher> = {}): SecretCipher {
  return {
    available: () => Promise.resolve(true),
    encrypt: (text) => Promise.resolve(Buffer.from(reversed(text))),
    decrypt: (data) => Promise.resolve({ text: reversed(data.toString()), stale: false }),
    ...overrides,
  };
}

void describe("isKeyringCiphertext", () => {
  void it("takes a keyring's format alone", () => {
    assert.equal(isKeyringCiphertext(Buffer.from("v11sealed")), true);
    assert.equal(isKeyringCiphertext(Buffer.from("v10sealed")), false);
    assert.equal(isKeyringCiphertext(Buffer.from("")), false);
  });
});

void describe("SecretFile", () => {
  let directory: string;
  let count = 0;
  const path = () => join(directory, `file-${String((count += 1))}`);

  before(async () => {
    directory = await mkdtemp(join(tmpdir(), "aspen-secret-file-"));
  });
  after(async () => {
    await rm(directory, { recursive: true, force: true });
  });

  void it("reads nothing before a write, then what was written, never as plain text", async () => {
    const target = path();
    const file = new SecretFile(target, reversing());
    assert.deepEqual(await file.read(), { status: "ok", contents: null });
    assert.equal(await file.write("token"), true);
    assert.equal((await readFile(target)).toString(), "nekot");
    assert.deepEqual(await file.read(), { status: "ok", contents: "token" });
    if (process.platform !== "win32") {
      assert.equal((await stat(target)).mode & 0o777, 0o600);
    }
  });

  void it("neither reads nor writes without the system's encryption", async () => {
    const target = path();
    const file = new SecretFile(target, reversing({ available: () => Promise.resolve(false) }));
    assert.deepEqual(await file.read(), { status: "unavailable" });
    assert.equal(await file.write("token"), false);
    await assert.rejects(readFile(target));
  });

  void it("calls contents the system will not open unreadable, and leaves them", async () => {
    const target = path();
    await writeFile(target, "sealed");
    const file = new SecretFile(
      target,
      reversing({ decrypt: () => Promise.reject(new Error("locked")) }),
    );
    assert.deepEqual(await file.read(), { status: "unreadable" });
    assert.equal((await readFile(target)).toString(), "sealed");
  });

  void it("keeps the old contents and no stray file when encrypting fails", async () => {
    const target = path();
    await new SecretFile(target, reversing()).write("old");
    const failing = new SecretFile(
      target,
      reversing({ encrypt: () => Promise.reject(new Error("no")) }),
    );
    assert.equal(await failing.write("new"), false);
    assert.equal((await readFile(target)).toString(), "dlo");
    const strays = (await readdir(directory)).filter((name) => name.startsWith(`${target}.`));
    assert.deepEqual(strays, []);
  });

  void it("writes one at a time, the last call's contents staying", async () => {
    const target = path();
    const file = new SecretFile(target, reversing());
    await Promise.all(["a", "b", "c"].map((text) => file.write(text)));
    assert.deepEqual(await file.read(), { status: "ok", contents: "c" });
  });

  void it("writes again what the system says it would now encrypt otherwise", async () => {
    const target = path();
    await writeFile(target, "dlo");
    let decrypted = 0;
    const file = new SecretFile(
      target,
      reversing({
        decrypt: (data) => {
          decrypted += 1;
          return Promise.resolve({ text: reversed(data.toString()), stale: true });
        },
        encrypt: (text) => Promise.resolve(Buffer.from(`v2:${text}`)),
      }),
    );
    assert.deepEqual(await file.read(), { status: "ok", contents: "old" });
    assert.equal(decrypted, 1);
    assert.equal((await readFile(target)).toString(), "v2:old");
  });
});
