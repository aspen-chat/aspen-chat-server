import type { FileSink, OfferState, TransferMode } from "@aspen/protocol";
import { nativeCanChooseDestination, nativeChooseDestination } from "@/api/filesBridge";

/** How long an offer may stand, in seconds, as the offer dialog lists them; the first is the default. */
export const VALIDITY_CHOICES = ["60", "300", "900", "1800", "3600"] as const;
export type Validity = (typeof VALIDITY_CHOICES)[number];

const UNITS = ["byte", "kilobyte", "megabyte", "gigabyte", "terabyte"] as const;

/** A size in bytes in the largest unit that keeps it at 1 or more, to one decimal place. */
export function formatSize(bytes: number, locale: string): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return new Intl.NumberFormat(locale, {
    style: "unit",
    unit: UNITS[unit],
    unitDisplay: unit === 0 ? "narrow" : "short",
    maximumFractionDigits: unit === 0 ? 0 : 1,
  }).format(value);
}

/** Time left as `m:ss` under an hour, and `h:mm:ss` from there. */
export function formatTimeLeft(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = String(total % 60).padStart(2, "0");
  return hours > 0
    ? `${String(hours)}:${String(minutes).padStart(2, "0")}:${seconds}`
    : `${String(minutes)}:${seconds}`;
}

/** The ways an offer can reach this user: direct when its sender allows it, relayed when the server relays. */
export function receiveModes(offer: OfferState, relayMbps: number | null): TransferMode[] {
  return [
    ...(offer.allowDirect ? (["directPreferred"] as const) : []),
    ...(relayMbps === null ? [] : (["relayOnly"] as const)),
  ];
}

/** A tile's box, relative to the drawing the links are in. */
export interface Box {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

/**
 * The route of a link from one tile to another as an SVG path of straight runs and right-angle
 * turns: out of the sender's bottom (or top, when the receiver is above), along the gap between
 * rows, and into the receiver. Tiles in one row are joined a gap's width below it. `lane` shifts the runs
 * sideways and along the gap, so that several links between the same tiles do not overlap.
 */
/** A coordinate as path data writes it: to a tenth of a pixel. */
function at(value: number): string {
  return String(Math.round(value * 10) / 10);
}

export function linkPath(from: Box, to: Box, gap: number, lane = 0): string {
  const shift = lane * 6;
  const fromX = from.left + from.width / 2 + shift;
  const toX = to.left + to.width / 2 + shift;
  const fromBottom = from.top + from.height;
  const toBottom = to.top + to.height;
  const sameRow = Math.abs(from.top - to.top) < Math.min(from.height, to.height) / 2;
  if (sameRow) {
    const below = Math.max(fromBottom, toBottom) + gap + shift / 3;
    return `M${at(fromX)},${at(fromBottom)} V${at(below)} H${at(toX)} V${at(toBottom)}`;
  }
  if (to.top > from.top) {
    const between = (fromBottom + to.top) / 2 + shift / 3;
    return `M${at(fromX)},${at(fromBottom)} V${at(between)} H${at(toX)} V${at(to.top)}`;
  }
  const between = (from.top + toBottom) / 2 + shift / 3;
  return `M${at(fromX)},${at(from.top)} V${at(between)} H${at(toX)} V${at(toBottom)}`;
}

/** A name safe to save a received file under: no directories, and no control characters. */
export function safeFileName(name: string): string {
  const cleaned = name
    .replace(/[/\\]/g, "_")
    .replace(/\p{Cc}/gu, "")
    .trim();
  return cleaned.length > 0 ? cleaned : "file";
}

interface SavePicker {
  showSaveFilePicker?: (options: { suggestedName: string }) => Promise<FileSystemFileHandle>;
}

/**
 * Whether the receiver can choose where a file goes before it arrives: in the Android app,
 * through the system's picker (`filesBridge`), and in a browser with the File System Access API
 * (Chromium, and so the desktop app). Elsewhere a file is held until it has all arrived and then
 * saved.
 */
export function canChooseDestination(): boolean {
  return (
    nativeCanChooseDestination() || typeof (window as SavePicker).showSaveFilePicker === "function"
  );
}

/**
 * Asks where to save `name` and opens the file for writing; `null` when the receiver closed the
 * picker without choosing. Must be called from a click, which the picker needs.
 */
export async function chooseDestination(name: string): Promise<FileSink | null> {
  if (nativeCanChooseDestination()) {
    return nativeChooseDestination(safeFileName(name));
  }
  const picker = (window as SavePicker).showSaveFilePicker;
  if (picker === undefined) {
    return null;
  }
  try {
    const handle = await picker({ suggestedName: safeFileName(name) });
    return await handle.createWritable();
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      return null;
    }
    throw error;
  }
}
