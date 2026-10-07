import { loadFamily } from "./fontLibrary";

/**
 * The typefaces Aspen draws in: one for text and one for code. Each is Aspen's own (Inclusive
 * Sans for text, Intel One Mono for code, bundled with the app) unless the user chose a family
 * from their font library (`fontLibrary.ts`). The choice is remembered per install, like the
 * palette, and never follows the account to another device, where the files are not.
 *
 * `styles.css` builds `--font-sans` and `--font-mono` on `--font-text-chosen` and
 * `--font-code-chosen`, falling back to the bundled face and then the system's, so characters a
 * chosen font lacks are still drawn.
 */

export const FONT_ROLES = ["text", "code"] as const;
export type FontRole = (typeof FONT_ROLES)[number];

const STORAGE_KEYS: Record<FontRole, string> = {
  text: "aspen.font.text",
  code: "aspen.font.code",
};

const VARIABLES: Record<FontRole, string> = {
  text: "--font-text-chosen",
  code: "--font-code-chosen",
};

/** The family chosen for `role`, or `null` for Aspen's own. */
export function storedFont(role: FontRole): string | null {
  try {
    return window.localStorage.getItem(STORAGE_KEYS[role]);
  } catch {
    return null;
  }
}

function remember(role: FontRole, family: string | null): void {
  try {
    if (family === null) {
      window.localStorage.removeItem(STORAGE_KEYS[role]);
    } else {
      window.localStorage.setItem(STORAGE_KEYS[role], family);
    }
  } catch {
    // Not fatal; Aspen's own font applies next launch.
  }
}

/** The aliases of the user's families drawn now, for text and for code (`applyFont`). */
export function chosenAliases(): string[] {
  const style = document.documentElement.style;
  return FONT_ROLES.map((role) =>
    style.getPropertyValue(VARIABLES[role]).trim().replace(/^"|"$/g, ""),
  ).filter((alias) => alias !== "");
}

/** A `font-family` value for showing a family by its alias, as Settings lists them. */
export function aliasStack(alias: string, role: FontRole): string {
  return `"${alias}", ${role === "text" ? "sans-serif" : "monospace"}`;
}

const generation: Record<FontRole, number> = { text: 0, code: 0 };

/**
 * Draws `role` in `family` (or Aspen's own font for `null`) and remembers it for the next
 * launch. A family no longer in the library, or none of whose files the browser can draw,
 * falls back to Aspen's own and is forgotten. Resolves to the family that applied, or to
 * `undefined` when a later choice for `role` was made while this one loaded, which wins.
 * Rejects when the library cannot be read.
 */
export async function applyFont(
  role: FontRole,
  family: string | null,
): Promise<string | null | undefined> {
  const mine = ++generation[role];
  // A library that cannot be read rejects here, leaving the remembered choice for next launch.
  const alias = family === null ? undefined : await loadFamily(family);
  if (mine !== generation[role]) {
    return undefined;
  }
  const style = document.documentElement.style;
  if (alias === undefined) {
    style.removeProperty(VARIABLES[role]);
    remember(role, null);
    return null;
  }
  style.setProperty(VARIABLES[role], `"${alias}"`);
  remember(role, family);
  return family;
}

/** Applies the fonts remembered for this install, at startup. */
export async function applyStoredFonts(): Promise<void> {
  await Promise.all(
    FONT_ROLES.map(async (role) => {
      const family = storedFont(role);
      if (family !== null) {
        await applyFont(role, family).catch((error: unknown) => {
          console.warn(`the ${role} font could not be loaded`, error);
        });
      }
    }),
  );
}
