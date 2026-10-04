/**
 * Reads what a font file says about itself: its family, the weight or range of weights it
 * draws, and whether it is upright or slanted, so that files the user adds group into families
 * and each is registered with the `@font-face` descriptors that let the browser pick the right
 * one for bold and italic text. TrueType and OpenType files (`.ttf`, `.otf`) are read directly
 * and WOFF files after inflating the tables needed; WOFF2 packs every table into one Brotli
 * stream, which browsers do not offer to decompress, and font collections (`.ttc`) hold several
 * faces a `FontFace` cannot choose between, so both are refused with a reason.
 */

export type FontStyle = "normal" | "italic" | "oblique";

export interface FontInfo {
  /** The typographic family (name 16), or the legacy family (name 1) where it has none. */
  family: string;
  /** One weight, or the bounds of a variable font's weight axis. */
  weight: number | readonly [number, number];
  style: FontStyle;
}

export type FontFileProblem = "woff2" | "collection" | "unrecognized" | "unnamed";

export class FontFileError extends Error {
  constructor(readonly problem: FontFileProblem) {
    super(`font file not readable: ${problem}`);
    this.name = "FontFileError";
  }
}

const SFNT_TRUETYPE = 0x00010000;
const SFNT_APPLE_TRUE = 0x74727565; // "true"
const SFNT_OPENTYPE = 0x4f54544f; // "OTTO"
const WOFF = 0x774f4646; // "wOFF"
const WOFF2 = 0x774f4632; // "wOF2"
const COLLECTION = 0x74746366; // "ttcf"

const NEEDED_TABLES = ["name", "OS/2", "fvar"] as const;
type TableTag = (typeof NEEDED_TABLES)[number];
type Tables = Partial<Record<TableTag, DataView>>;

/** What `bytes`, the whole of a font file, says about the face it holds. */
export async function readFontInfo(bytes: ArrayBuffer): Promise<FontInfo> {
  const tables = await readTables(bytes);
  const family = tables.name === undefined ? undefined : familyName(tables.name);
  if (family === undefined) {
    throw new FontFileError("unnamed");
  }
  // `usWeightClass` is at offset 4 and `fsSelection` at 62 in every version of OS/2.
  const os2 = (tables["OS/2"]?.byteLength ?? 0) >= 64 ? tables["OS/2"] : undefined;
  const axis = tables.fvar === undefined ? undefined : weightAxis(tables.fvar);
  return {
    family,
    weight: axis ?? (os2 === undefined ? 400 : clampWeight(os2.getUint16(4))),
    style: os2 === undefined ? "normal" : styleOf(os2.getUint16(62)),
  };
}

async function readTables(bytes: ArrayBuffer): Promise<Tables> {
  const view = new DataView(bytes);
  if (view.byteLength < 12) {
    throw new FontFileError("unrecognized");
  }
  const signature = view.getUint32(0);
  switch (signature) {
    case SFNT_TRUETYPE:
    case SFNT_APPLE_TRUE:
    case SFNT_OPENTYPE:
      return sfntTables(view);
    case WOFF:
      return woffTables(view);
    case WOFF2:
      throw new FontFileError("woff2");
    case COLLECTION:
      throw new FontFileError("collection");
    default:
      throw new FontFileError("unrecognized");
  }
}

function tagAt(view: DataView, offset: number): string {
  return String.fromCharCode(
    view.getUint8(offset),
    view.getUint8(offset + 1),
    view.getUint8(offset + 2),
    view.getUint8(offset + 3),
  );
}

function isNeeded(tag: string): tag is TableTag {
  return (NEEDED_TABLES as readonly string[]).includes(tag);
}

/** A slice of `view`, refusing one that runs past the end of the file. */
function slice(view: DataView, offset: number, length: number): DataView {
  if (offset + length > view.byteLength) {
    throw new FontFileError("unrecognized");
  }
  return new DataView(view.buffer, view.byteOffset + offset, length);
}

/** The table directory of a TrueType or OpenType file: 16 bytes a table, after a 12-byte header. */
function sfntTables(view: DataView): Tables {
  const count = view.getUint16(4);
  const tables: Tables = {};
  for (let i = 0; i < count; i++) {
    const record = 12 + i * 16;
    const tag = tagAt(slice(view, record, 16), 0);
    if (isNeeded(tag)) {
      tables[tag] = slice(view, view.getUint32(record + 8), view.getUint32(record + 12));
    }
  }
  return tables;
}

/**
 * The table directory of a WOFF file: 20 bytes a table, after a 44-byte header, each table
 * zlib-compressed unless its compressed length equals its original length.
 */
async function woffTables(view: DataView): Promise<Tables> {
  const count = view.getUint16(12);
  const tables: Tables = {};
  for (let i = 0; i < count; i++) {
    const record = 44 + i * 20;
    const tag = tagAt(slice(view, record, 20), 0);
    if (!isNeeded(tag)) {
      continue;
    }
    const offset = view.getUint32(record + 4);
    const compressed = view.getUint32(record + 8);
    const original = view.getUint32(record + 12);
    const data = slice(view, offset, compressed);
    tables[tag] = compressed === original ? data : await inflate(data, original);
  }
  return tables;
}

async function inflate(data: DataView, length: number): Promise<DataView> {
  const stream = new DecompressionStream("deflate");
  const writer = stream.writable.getWriter();
  const compressed = new Uint8Array(data.byteLength);
  compressed.set(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
  void writer.write(compressed).catch(() => undefined);
  void writer.close().catch(() => undefined);
  let output: ArrayBuffer;
  try {
    output = await new Response(stream.readable).arrayBuffer();
  } catch {
    throw new FontFileError("unrecognized");
  }
  if (output.byteLength !== length) {
    throw new FontFileError("unrecognized");
  }
  return new DataView(output);
}

const TYPOGRAPHIC_FAMILY = 16;
const FAMILY = 1;
const PLATFORM_MAC = 1;
const PLATFORM_WINDOWS = 3;
const WINDOWS_ENGLISH = 0x409;

/**
 * The family name, preferring the typographic family (which groups every weight under one
 * name, where the legacy family splits off "Light" and "Black"), and among its records the
 * Windows English one, then any Windows one, then a Macintosh Roman one.
 */
function familyName(name: DataView): string | undefined {
  const count = name.getUint16(2);
  const storage = name.getUint16(4);
  let best: { rank: number; value: string } | undefined;
  for (let i = 0; i < count; i++) {
    const record = 6 + i * 12;
    if (record + 12 > name.byteLength) {
      break;
    }
    const platform = name.getUint16(record);
    const encoding = name.getUint16(record + 2);
    const language = name.getUint16(record + 4);
    const id = name.getUint16(record + 6);
    if (id !== TYPOGRAPHIC_FAMILY && id !== FAMILY) {
      continue;
    }
    const windows = platform === PLATFORM_WINDOWS && (encoding === 1 || encoding === 10);
    const mac = platform === PLATFORM_MAC && encoding === 0;
    if (!windows && !mac) {
      continue;
    }
    const rank =
      (id === TYPOGRAPHIC_FAMILY ? 0 : 3) + (windows ? (language === WINDOWS_ENGLISH ? 0 : 1) : 2);
    if (best !== undefined && best.rank <= rank) {
      continue;
    }
    const length = name.getUint16(record + 8);
    const offset = storage + name.getUint16(record + 10);
    const value = (
      windows ? utf16(slice(name, offset, length)) : macRoman(slice(name, offset, length))
    ).trim();
    if (value !== "") {
      best = { rank, value };
    }
  }
  return best?.value;
}

function utf16(view: DataView): string {
  const units: number[] = [];
  for (let i = 0; i + 1 < view.byteLength; i += 2) {
    units.push(view.getUint16(i));
  }
  return String.fromCharCode(...units);
}

/** Macintosh Roman; family names in it are ASCII in practice, and other bytes become U+FFFD. */
function macRoman(view: DataView): string {
  let text = "";
  for (let i = 0; i < view.byteLength; i++) {
    const byte = view.getUint8(i);
    text += byte < 0x80 ? String.fromCharCode(byte) : "�";
  }
  return text;
}

/** The bounds of the `wght` axis of a variable font's `fvar` table, if it has one. */
function weightAxis(fvar: DataView): readonly [number, number] | undefined {
  const axesOffset = fvar.getUint16(4);
  const count = fvar.getUint16(8);
  const size = fvar.getUint16(10);
  for (let i = 0; i < count; i++) {
    const axis = axesOffset + i * size;
    if (axis + 20 > fvar.byteLength) {
      break;
    }
    if (tagAt(fvar, axis) === "wght") {
      const min = clampWeight(fvar.getInt32(axis + 4) / 65536);
      const max = clampWeight(fvar.getInt32(axis + 12) / 65536);
      return min <= max ? [min, max] : [max, min];
    }
  }
  return undefined;
}

/** CSS accepts weights from 1 to 1000. */
function clampWeight(weight: number): number {
  return Math.min(1000, Math.max(1, Math.round(weight)));
}

/** `fsSelection` in the OS/2 table: bit 0 is italic, bit 9 oblique. */
function styleOf(selection: number): FontStyle {
  if ((selection & 0x1) !== 0) {
    return "italic";
  }
  return (selection & 0x200) !== 0 ? "oblique" : "normal";
}
