import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { describe, expect, it, beforeAll } from "vitest";
import { prepareZXingModule, readBarcodes } from "zxing-wasm/reader";
import { qrDrawing, qrSvgDocument, svgContents } from "./qrDrawing";

const PIXELS_PER_MODULE = 4;

/**
 * The code's modules as a binary PGM image, the patch left light as drawn: the mark's strokes
 * are left out, so the scanner gets no help from them.
 */
function raster(text: string): Uint8Array {
  const { extent, modules } = qrDrawing(text);
  const side = extent * PIXELS_PER_MODULE;
  const pixels = new Uint8Array(side * side).fill(255);
  for (const match of modules.matchAll(/M(\d+) (\d+)/g)) {
    const column = Number(match[1]);
    const row = Number(match[2]);
    for (let y = 0; y < PIXELS_PER_MODULE; y++) {
      const start = (row * PIXELS_PER_MODULE + y) * side + column * PIXELS_PER_MODULE;
      pixels.fill(0, start, start + PIXELS_PER_MODULE);
    }
  }
  const header = new TextEncoder().encode(`P5\n${String(side)} ${String(side)}\n255\n`);
  const image = new Uint8Array(header.length + pixels.length);
  image.set(header);
  image.set(pixels, header.length);
  return image;
}

beforeAll(async () => {
  const wasm = createRequire(import.meta.url).resolve("zxing-wasm/reader/zxing_reader.wasm");
  const binary = readFileSync(wasm);
  await prepareZXingModule({
    overrides: {
      wasmBinary: binary.buffer.slice(binary.byteOffset, binary.byteOffset + binary.byteLength),
    },
    fireImmediately: true,
  });
});

describe("qrDrawing", () => {
  const texts = [
    "https://chat.example.org/invite/AbCdEfGhIjKlMnOp?at=chat.example.org:8443",
    "https://chat.example.org/register?invite=AbCdEfGhIjKlMnOp",
    `https://chat.example.org/device-link?server=${encodeURIComponent("https://api.chat.example.org")}#${"x".repeat(43)}`,
    "aspen://app/invite/abc123",
  ];

  it.each(texts)("still scans with the middle left for the mark: %s", async (text) => {
    const [found] = await readBarcodes(raster(text), { formats: ["QRCode"], tryHarder: false });
    expect(found?.text).toBe(text);
  });

  it("centres the patch and the mark on the module grid", () => {
    const { extent, patch, mark } = qrDrawing(texts[0] ?? "");
    expect(patch.x * 2 + patch.size).toBe(extent);
    expect(Number.isInteger(patch.x)).toBe(true);
    expect(mark.y * 2 + mark.height).toBeCloseTo(extent);
    expect(mark.x * 2 + mark.width).toBeCloseTo(extent);
    expect(mark.height).toBe(patch.size - 2);
  });

  it("writes a document with the mark inside and the title escaped", () => {
    const svg = qrSvgDocument(qrDrawing("hello"), 'Invite <"x"> & co', 512);
    expect(svg).toContain('width="512"');
    expect(svg).toContain("<path d=");
    expect(svg).toContain('stroke="#000000"');
    expect(svg).toContain("Invite &lt;&quot;x&quot;> &amp; co");
  });

  it("writes well-formed XML holding the mark's drawing in one inner svg", () => {
    const svg = qrSvgDocument(qrDrawing("hello"), "Invite", 512);
    const parsed = new DOMParser().parseFromString(svg, "image/svg+xml");
    expect(parsed.querySelector("parsererror")).toBeNull();
    expect(parsed.querySelectorAll("svg")).toHaveLength(2);
    expect(parsed.querySelectorAll("svg svg path").length).toBeGreaterThan(0);
    // The mark's own title would name the whole code a second time.
    expect(parsed.querySelectorAll("title")).toHaveLength(1);
  });
});

describe("svgContents", () => {
  const root = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1">';

  it("takes what is inside the root, with or without a title", () => {
    expect(svgContents(`${root}<title>Aspen</title><g><path d="M0 0"/></g></svg>\n`)).toBe(
      '<g><path d="M0 0"/></g>',
    );
    expect(svgContents(`<?xml version="1.0"?>\n${root}<g><path d="M0 0"/></g></svg>`)).toBe(
      '<g><path d="M0 0"/></g>',
    );
  });

  it("refuses what is not one root svg", () => {
    expect(() => svgContents("<g/>")).toThrow();
    expect(() => svgContents(`${root}<g/>`)).toThrow();
  });
});
