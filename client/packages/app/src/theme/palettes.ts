/**
 * Colour palettes. Each one is a block of CSS variables in `styles.css` selected by the
 * `data-theme` attribute on <html>; this module only chooses which block applies and remembers
 * the choice per browser.
 */

export const PALETTES = ["aspen", "dusk"] as const;
export type Palette = (typeof PALETTES)[number];

const STORAGE_KEY = "aspen.palette";
const DEFAULT_PALETTE: Palette = "aspen";

export function isPalette(value: unknown): value is Palette {
  return typeof value === "string" && (PALETTES as readonly string[]).includes(value);
}

export function storedPalette(): Palette {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return isPalette(stored) ? stored : DEFAULT_PALETTE;
  } catch {
    return DEFAULT_PALETTE;
  }
}

/** Applies a palette to the document and remembers it for the next launch. */
export function applyPalette(palette: Palette): void {
  document.documentElement.dataset.theme = palette;
  try {
    window.localStorage.setItem(STORAGE_KEY, palette);
  } catch {
    // Not fatal; the default palette applies next launch.
  }
}
