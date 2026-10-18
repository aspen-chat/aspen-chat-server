/**
 * Device credentials: what a deployment counts a client that has not signed in by, so that the
 * many people behind one address each have limits of their own. `AspenClient` presents one on
 * every request it makes without a session (`DEVICE_HEADER`), keeps the fresh one a server
 * answers with when its own is past half its life (`RENEWAL_HEADER`), and earns one when a
 * server answers `deviceCredentialRequired`.
 *
 * Earning one takes solving a puzzle: a nonce for which `SHA-256(SHA-256(challenge) || nonce)`,
 * with the nonce as eight big-endian bytes, begins with the challenge's number of zero bits,
 * about a million hashes at the usual twenty. The solver here passes the vectors the server's
 * does (`spec/device_puzzle_vectors.json`).
 */

export const DEVICE_HEADER = "aspen-device";
export const RENEWAL_HEADER = "aspen-device-credential";

/**
 * Where a deployment's device credential is kept. It outlives sign-outs: it says which device
 * this is, not who uses it.
 */
export interface DeviceCredentialStore {
  load(): string | null;
  save(credential: string): void;
}

export class MemoryDeviceCredentialStore implements DeviceCredentialStore {
  #credential: string | null = null;

  load(): string | null {
    return this.#credential;
  }

  save(credential: string): void {
    this.#credential = credential;
  }
}

/** Keeps the credential under one key of a `Storage` (localStorage). */
export class WebStorageDeviceCredentialStore implements DeviceCredentialStore {
  readonly #storage: Storage;
  readonly #key: string;

  constructor(storage: Storage, key = "aspen.device") {
    this.#storage = storage;
    this.#key = key;
  }

  load(): string | null {
    try {
      return this.#storage.getItem(this.#key);
    } catch {
      return null;
    }
  }

  save(credential: string): void {
    try {
      this.#storage.setItem(this.#key, credential);
    } catch {
      // A full or refused store leaves the credential to this run: the next earns another.
    }
  }
}

/** Finds a nonce solving a challenge, given as its decoded bytes. */
export type PuzzleSolver = (challenge: Uint8Array, difficulty: number) => Promise<number>;

const K = Uint32Array.from([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

const IV = Uint32Array.from([
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
]);

/**
 * Runs SHA-256's compression of the block in `w[0..16]` into `state`, using the rest of `w`
 * for the message schedule. Plain integer arithmetic rather than WebCrypto, whose every call is
 * asynchronous: a puzzle takes a million of them.
 */
function compress(state: Uint32Array, w: Uint32Array): void {
  for (let i = 16; i < 64; i++) {
    const a = w[i - 15] ?? 0;
    const b = w[i - 2] ?? 0;
    const s0 = ((a >>> 7) | (a << 25)) ^ ((a >>> 18) | (a << 14)) ^ (a >>> 3);
    const s1 = ((b >>> 17) | (b << 15)) ^ ((b >>> 19) | (b << 13)) ^ (b >>> 10);
    w[i] = ((w[i - 16] ?? 0) + s0 + (w[i - 7] ?? 0) + s1) | 0;
  }
  let [a = 0, b = 0, c = 0, d = 0, e = 0, f = 0, g = 0, h = 0] = state;
  for (let i = 0; i < 64; i++) {
    const s1 = ((e >>> 6) | (e << 26)) ^ ((e >>> 11) | (e << 21)) ^ ((e >>> 25) | (e << 7));
    const t1 = (h + s1 + ((e & f) ^ (~e & g)) + (K[i] ?? 0) + (w[i] ?? 0)) | 0;
    const s0 = ((a >>> 2) | (a << 30)) ^ ((a >>> 13) | (a << 19)) ^ ((a >>> 22) | (a << 10));
    const t2 = (s0 + ((a & b) ^ (a & c) ^ (b & c))) | 0;
    h = g;
    g = f;
    f = e;
    e = (d + t1) | 0;
    d = c;
    c = b;
    b = a;
    a = (t1 + t2) | 0;
  }
  state[0] = ((state[0] ?? 0) + a) | 0;
  state[1] = ((state[1] ?? 0) + b) | 0;
  state[2] = ((state[2] ?? 0) + c) | 0;
  state[3] = ((state[3] ?? 0) + d) | 0;
  state[4] = ((state[4] ?? 0) + e) | 0;
  state[5] = ((state[5] ?? 0) + f) | 0;
  state[6] = ((state[6] ?? 0) + g) | 0;
  state[7] = ((state[7] ?? 0) + h) | 0;
}

/** SHA-256 of `bytes`, as its eight big-endian words. */
function sha256(bytes: Uint8Array): Uint32Array {
  const blocks = Math.ceil((bytes.length + 9) / 64);
  const padded = new Uint8Array(blocks * 64);
  padded.set(bytes);
  padded[bytes.length] = 0x80;
  const view = new DataView(padded.buffer);
  const bits = bytes.length * 8;
  view.setUint32(padded.length - 8, Math.floor(bits / 2 ** 32));
  view.setUint32(padded.length - 4, bits >>> 0);
  const state = IV.slice();
  const w = new Uint32Array(64);
  for (let block = 0; block < blocks; block++) {
    for (let i = 0; i < 16; i++) {
      w[i] = view.getUint32(block * 64 + i * 4);
    }
    compress(state, w);
  }
  return state;
}

/** Hashes `SHA-256(challenge) || nonce` for one nonce after another, sharing its buffers. */
class Puzzle {
  readonly #prefix: Uint32Array;
  readonly #state = new Uint32Array(8);
  readonly #w = new Uint32Array(64);

  constructor(challenge: Uint8Array) {
    this.#prefix = sha256(challenge);
  }

  /** The leading zero bits `nonce` earns. */
  work(nonce: number): number {
    const w = this.#w;
    w.set(this.#prefix);
    w[8] = Math.floor(nonce / 2 ** 32);
    w[9] = nonce >>> 0;
    // The 40-byte message's padding: one bit, then its length in bits.
    w[10] = 0x80000000;
    w[11] = 0;
    w[12] = 0;
    w[13] = 0;
    w[14] = 0;
    w[15] = 320;
    const state = this.#state;
    state.set(IV);
    compress(state, w);
    let bits = 0;
    for (const word of state) {
      if (word !== 0) {
        return bits + Math.clz32(word);
      }
      bits += 32;
    }
    return bits;
  }
}

/** The leading zero bits `nonce` earns against `challenge`. */
export function puzzleWork(challenge: Uint8Array, nonce: number): number {
  return new Puzzle(challenge).work(nonce);
}

/** The first nonce at or after `from`, and before `to`, that earns `difficulty` bits. */
export function solvePuzzleRange(
  challenge: Uint8Array,
  difficulty: number,
  from: number,
  to: number,
): number | null {
  const puzzle = new Puzzle(challenge);
  for (let nonce = from; nonce < to; nonce++) {
    if (puzzle.work(nonce) >= difficulty) {
      return nonce;
    }
  }
  return null;
}

/** How many nonces `solvePuzzle` tries between letting other work run. */
const SLICE = 1 << 15;

/**
 * Solves a puzzle on the calling thread, a slice of nonces at a time with a pause between, so a
 * page without a worker stays responsive, if slower.
 */
export const solvePuzzle: PuzzleSolver = async (challenge, difficulty) => {
  for (let from = 0; ; from += SLICE) {
    const found = solvePuzzleRange(challenge, difficulty, from, from + SLICE);
    if (found !== null) {
      return found;
    }
    await new Promise((resolve) => globalThis.setTimeout(resolve, 0));
  }
};

/** Decodes base64url without padding, as challenges are written. */
export function decodeChallenge(challenge: string): Uint8Array {
  const base64 = challenge.replaceAll("-", "+").replaceAll("_", "/");
  const binary = atob(base64 + "=".repeat((4 - (base64.length % 4)) % 4));
  return Uint8Array.from(binary, (char) => char.charCodeAt(0));
}
