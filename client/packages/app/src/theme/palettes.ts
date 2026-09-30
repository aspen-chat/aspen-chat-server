/**
 * Colour palettes and whether they are drawn light or dark. Each palette is a block of CSS
 * variables in `styles.css` selected by the `data-theme` attribute on <html>, every colour in it
 * a `light-dark()` pair; the mode sets the document's `color-scheme`, which picks one of each
 * pair. This module only chooses and remembers both, per browser.
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

/** Whether the palette is drawn light or dark, or as the system prefers. */
export const THEME_MODES = ["system", "light", "dark"] as const;
export type ThemeMode = (typeof THEME_MODES)[number];

const MODE_KEY = "aspen.themeMode";

export function isThemeMode(value: unknown): value is ThemeMode {
  return typeof value === "string" && (THEME_MODES as readonly string[]).includes(value);
}

export function storedThemeMode(): ThemeMode {
  try {
    const stored = window.localStorage.getItem(MODE_KEY);
    return isThemeMode(stored) ? stored : "system";
  } catch {
    return "system";
  }
}

/**
 * Applies a mode to the document and remembers it for the next launch: the root's
 * `color-scheme`, which every `light-dark()` colour follows, and the `color-scheme` meta tag,
 * which the browser's own controls and scroll bars follow. `system` offers both, leaving the
 * choice to the system's preference.
 */
export function applyThemeMode(mode: ThemeMode): void {
  const scheme = mode === "system" ? "light dark" : mode;
  document.documentElement.style.colorScheme = scheme;
  document.querySelector('meta[name="color-scheme"]')?.setAttribute("content", scheme);
  try {
    window.localStorage.setItem(MODE_KEY, mode);
  } catch {
    // Not fatal; the system's preference applies next launch.
  }
}
