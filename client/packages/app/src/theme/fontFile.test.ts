import { describe, expect, it } from "vitest";
import { FontFileError, readFontInfo } from "./fontFile";

/** The UTF-16 code units of `text`, which every name and tag here is written in. */
function codes(text: string): number[] {
  return Array.from({ length: text.length }, (_, i) => text.charCodeAt(i));
}

/** A `name` table holding each of `records` as Windows UTF-16 (or Macintosh Roman, platform 1). */
function nameTable(
  records: readonly { id: number; value: string; platform?: number; language?: number }[],
): Uint8Array {
  const strings = records.map(({ value, platform }) =>
    platform === 1
      ? Uint8Array.from(codes(value))
      : Uint8Array.from(codes(value).flatMap((c) => [c >> 8, c & 0xff])),
  );
  const storage = 6 + records.length * 12;
  const table = new Uint8Array(storage + strings.reduce((sum, s) => sum + s.length, 0));
  const view = new DataView(table.buffer);
  view.setUint16(2, records.length);
  view.setUint16(4, storage);
  let offset = 0;
  records.forEach(({ id, platform = 3, language = 0x409 }, i) => {
    const record = 6 + i * 12;
    const bytes = strings[i] ?? new Uint8Array();
    view.setUint16(record, platform);
    view.setUint16(record + 2, platform === 1 ? 0 : 1);
    view.setUint16(record + 4, platform === 1 ? 0 : language);
    view.setUint16(record + 6, id);
    view.setUint16(record + 8, bytes.length);
    view.setUint16(record + 10, offset);
    table.set(bytes, storage + offset);
    offset += bytes.length;
  });
  return table;
}

function os2Table(weight: number, selection: number): Uint8Array {
  const table = new Uint8Array(96);
  const view = new DataView(table.buffer);
  view.setUint16(0, 4);
  view.setUint16(4, weight);
  view.setUint16(62, selection);
  return table;
}

function fvarTable(min: number, max: number): Uint8Array {
  const table = new Uint8Array(16 + 2 * 20);
  const view = new DataView(table.buffer);
  view.setUint16(4, 16);
  view.setUint16(8, 2);
  view.setUint16(10, 20);
  const axis = (at: number, tag: string, from: number, to: number) => {
    codes(tag).forEach((c, i) => {
      view.setUint8(at + i, c);
    });
    view.setInt32(at + 4, from * 65536);
    view.setInt32(at + 12, to * 65536);
  };
  axis(16, "wdth", 75, 100);
  axis(36, "wght", min, max);
  return table;
}

type Tables = Record<string, Uint8Array>;

function sfnt(tables: Tables, signature = 0x00010000): ArrayBuffer {
  const entries = Object.entries(tables);
  let offset = 12 + entries.length * 16;
  const size = entries.reduce((sum, [, data]) => sum + data.length, offset);
  const file = new Uint8Array(size);
  const view = new DataView(file.buffer);
  view.setUint32(0, signature);
  view.setUint16(4, entries.length);
  entries.forEach(([tag, data], i) => {
    const record = 12 + i * 16;
    codes(tag).forEach((c, j) => {
      view.setUint8(record + j, c);
    });
    view.setUint32(record + 8, offset);
    view.setUint32(record + 12, data.length);
    file.set(data, offset);
    offset += data.length;
  });
  return file.buffer;
}

async function deflate(data: Uint8Array): Promise<Uint8Array> {
  const stream = new CompressionStream("deflate");
  const writer = stream.writable.getWriter();
  void writer.write(new Uint8Array(data));
  void writer.close();
  return new Uint8Array(await new Response(stream.readable).arrayBuffer());
}

async function woff(tables: Tables): Promise<ArrayBuffer> {
  const entries = await Promise.all(
    Object.entries(tables).map(async ([tag, data]) => [tag, data, await deflate(data)] as const),
  );
  let offset = 44 + entries.length * 20;
  const size = entries.reduce((sum, [, , packed]) => sum + packed.length, offset);
  const file = new Uint8Array(size);
  const view = new DataView(file.buffer);
  view.setUint32(0, 0x774f4646);
  view.setUint16(12, entries.length);
  entries.forEach(([tag, data, packed], i) => {
    const record = 44 + i * 20;
    codes(tag).forEach((c, j) => {
      view.setUint8(record + j, c);
    });
    view.setUint32(record + 4, offset);
    view.setUint32(record + 8, packed.length);
    view.setUint32(record + 12, data.length);
    file.set(packed, offset);
    offset += packed.length;
  });
  return file.buffer;
}

describe("readFontInfo", () => {
  it("reads the family, weight, and style of a static face", async () => {
    const font = sfnt({
      name: nameTable([
        { id: 1, value: "Example Semibold" },
        { id: 16, value: "Example" },
      ]),
      "OS/2": os2Table(600, 0x1),
    });
    expect(await readFontInfo(font)).toEqual({ family: "Example", weight: 600, style: "italic" });
  });

  it("prefers English Windows names, then other Windows names, then Macintosh ones", async () => {
    const font = sfnt(
      {
        name: nameTable([
          { id: 1, value: "Mac Name", platform: 1 },
          { id: 1, value: "Nom", language: 0x40c },
          { id: 1, value: "Name" },
        ]),
      },
      0x4f54544f,
    );
    expect(await readFontInfo(font)).toEqual({ family: "Name", weight: 400, style: "normal" });
    const macOnly = sfnt({ name: nameTable([{ id: 1, value: "Mac Name", platform: 1 }]) });
    expect((await readFontInfo(macOnly)).family).toBe("Mac Name");
  });

  it("reads a variable font's weight axis and an oblique face", async () => {
    const font = sfnt({
      name: nameTable([{ id: 1, value: "Flex" }]),
      "OS/2": os2Table(400, 0x200),
      fvar: fvarTable(200, 800),
    });
    expect(await readFontInfo(font)).toEqual({
      family: "Flex",
      weight: [200, 800],
      style: "oblique",
    });
  });

  it("inflates the tables of a WOFF file", async () => {
    const font = await woff({
      name: nameTable([{ id: 1, value: "Packed" }]),
      "OS/2": os2Table(700, 0),
    });
    expect(await readFontInfo(font)).toEqual({ family: "Packed", weight: 700, style: "normal" });
  });

  it("refuses what it cannot read, saying why", async () => {
    const refusal = async (bytes: ArrayBuffer) =>
      readFontInfo(bytes).then(
        () => undefined,
        (error: unknown) => (error instanceof FontFileError ? error.problem : error),
      );
    const signed = (signature: number) => {
      const file = new ArrayBuffer(64);
      new DataView(file).setUint32(0, signature);
      return file;
    };
    expect(await refusal(signed(0x774f4632))).toBe("woff2");
    expect(await refusal(signed(0x74746366))).toBe("collection");
    expect(await refusal(signed(0x12345678))).toBe("unrecognized");
    expect(await refusal(new ArrayBuffer(4))).toBe("unrecognized");
    expect(await refusal(sfnt({ "OS/2": os2Table(400, 0) }))).toBe("unnamed");
    const truncated = sfnt({ name: nameTable([{ id: 1, value: "Cut" }]) }).slice(0, 40);
    expect(await refusal(truncated)).toBe("unrecognized");
  });
});
