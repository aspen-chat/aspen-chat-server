import encodeQR from "qr";
import lineMark from "../../../../../brand/aspen-mark-line.svg?raw";

/** Light modules around the code, as the QR standard asks for so a scanner finds its edge. */
const QUIET_ZONE = 4;
/**
 * The share of the code's width the middle patch takes, logo and margin. Error correction at
 * `high` restores up to 30% of the code; a patch this wide covers under 7%, which leaves room for
 * a code photographed at an angle, scuffed, or printed small.
 */
const PATCH_SHARE = 0.26;

/** The line mark's drawing: its view box and what is inside its `<svg>`. */
const MARK = (() => {
  const viewBox = /viewBox="([^"]+)"/.exec(lineMark)?.[1] ?? "0 0 176 202";
  const body = lineMark.replace(/^[\s\S]*?<\/title>/, "").replace(/<\/svg>\s*$/, "");
  const [, , width = 176, height = 202] = viewBox.split(/\s+/).map(Number);
  return { viewBox, body, aspect: width / height };
})();

/** A QR code laid out in module units, from which it is drawn on screen and saved. */
export interface QrDrawing {
  /** Modules across, quiet zone included. */
  readonly extent: number;
  /** One path of every dark module, each a unit square. */
  readonly modules: string;
  /** The square left light in the middle for the mark, and the mark's box inside it. */
  readonly patch: { x: number; size: number };
  readonly mark: { x: number; y: number; width: number; height: number };
}

/**
 * Lays out `text` as a QR code with Aspen's line mark in the middle (`brand/aspen-mark-line.svg`,
 * drawn by `scripts/export_icons.py`): the modules under the patch are left light, and error
 * correction at `high` restores them for the scanner.
 */
export function qrDrawing(text: string): QrDrawing {
  const cells = encodeQR(text, "raw", { ecc: "high", border: QUIET_ZONE });
  const extent = cells.length;
  const symbol = extent - 2 * QUIET_ZONE;
  let size = Math.round(symbol * PATCH_SHARE);
  // The patch sits on the module grid, centred, so its width has the symbol's parity.
  if ((symbol - size) % 2 !== 0) {
    size += 1;
  }
  const x = (extent - size) / 2;
  let modules = "";
  cells.forEach((row, rowIndex) => {
    row.forEach((dark, column) => {
      const inPatch = column >= x && column < x + size && rowIndex >= x && rowIndex < x + size;
      if (dark && !inPatch) {
        modules += `M${String(column)} ${String(rowIndex)}h1v1h-1z`;
      }
    });
  });
  // One module of light around the mark, so its strokes never meet a dark module.
  const height = size - 2;
  const width = height * MARK.aspect;
  return {
    extent,
    modules,
    patch: { x, size },
    mark: { x: (extent - width) / 2, y: (extent - height) / 2, width, height },
  };
}

/** The line mark's view box and inner markup, for drawing it inside a code. */
export const MARK_DRAWING = { viewBox: MARK.viewBox, body: MARK.body };

/**
 * The code as a standalone SVG document, black on white whatever the theme, `pixels` across:
 * what is saved, and what the PNG is rendered from.
 */
export function qrSvgDocument(drawing: QrDrawing, title: string, pixels: number): string {
  const { extent, modules, mark } = drawing;
  const escaped = title.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/"/g, "&quot;");
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${String(extent)} ${String(extent)}" ` +
    `width="${String(pixels)}" height="${String(pixels)}" shape-rendering="crispEdges" ` +
    `role="img" aria-label="${escaped}"><title>${escaped}</title>` +
    `<rect width="${String(extent)}" height="${String(extent)}" fill="#ffffff"/>` +
    `<path d="${modules}" fill="#000000"/>` +
    `<svg x="${String(mark.x)}" y="${String(mark.y)}" width="${String(mark.width)}" ` +
    `height="${String(mark.height)}" viewBox="${MARK.viewBox}" shape-rendering="geometricPrecision">` +
    `${MARK.body}</svg></svg>\n`
  );
}
