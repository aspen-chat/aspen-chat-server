import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { MIN_CONTRAST, contrast, hueColors, luminanceOf } from "./nameColors";

/** The grounds a name sits on: every palette's surfaces, and `accent-soft` behind mentions. */
const GROUND_TOKENS = [
  "surface",
  "surface-raised",
  "surface-sunken",
  "surface-hover",
  "accent-soft",
];

const styles = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), "..", "styles.css"),
  "utf8",
);

/** Each ground token's light and dark values, in every palette `styles.css` declares. */
function grounds(): { name: string; light: string; dark: string }[] {
  const found = [];
  for (const token of GROUND_TOKENS) {
    const pattern = new RegExp(
      `--palette-${token}: light-dark\\((#[0-9a-f]{6}), (#[0-9a-f]{6})\\);`,
      "g",
    );
    for (const [, light, dark] of styles.matchAll(pattern)) {
      if (light !== undefined && dark !== undefined) {
        found.push({ name: token, light, dark });
      }
    }
  }
  return found;
}

const HUES = Array.from({ length: 360 }, (_, hue) => hue);

describe("name colours", () => {
  it("finds the grounds of every palette", () => {
    expect(grounds().length).toBeGreaterThanOrEqual(GROUND_TOKENS.length * 2);
  });

  it("read on every ground, in both schemes, at every hue", () => {
    for (const ground of grounds()) {
      for (const hue of HUES) {
        const { light, dark } = hueColors(hue);
        expect(
          contrast(luminanceOf(light), luminanceOf(ground.light)),
          `hue ${String(hue)} (${light}) on light ${ground.name} ${ground.light}`,
        ).toBeGreaterThanOrEqual(MIN_CONTRAST);
        expect(
          contrast(luminanceOf(dark), luminanceOf(ground.dark)),
          `hue ${String(hue)} (${dark}) on dark ${ground.name} ${ground.dark}`,
        ).toBeGreaterThanOrEqual(MIN_CONTRAST);
      }
    }
  });

  it("are darker on light grounds and lighter on dark ones", () => {
    for (const hue of HUES) {
      const { light, dark } = hueColors(hue);
      expect(luminanceOf(light)).toBeLessThan(luminanceOf(dark));
    }
  });

  it("stay colours rather than greys", () => {
    for (const hue of HUES) {
      const { light, dark } = hueColors(hue);
      for (const hex of [light, dark]) {
        const channels = [1, 3, 5].map((at) => Number.parseInt(hex.slice(at, at + 2), 16));
        expect(
          Math.max(...channels) - Math.min(...channels),
          `hue ${String(hue)} ${hex}`,
        ).toBeGreaterThan(40);
      }
    }
  });

  it("are the colours HSV's hues name", () => {
    const strongest = (hex: string) => {
      const channel = (at: number) => Number.parseInt(hex.slice(at, at + 2), 16);
      const [r, g, b] = [channel(1), channel(3), channel(5)];
      return r >= g && r >= b ? "red" : g >= b ? "green" : "blue";
    };
    for (const [hue, named] of [
      [0, "red"],
      [120, "green"],
      [240, "blue"],
    ] as const) {
      const { light, dark } = hueColors(hue);
      expect(strongest(light), `hue ${String(hue)} light ${light}`).toBe(named);
      expect(strongest(dark), `hue ${String(hue)} dark ${dark}`).toBe(named);
    }
  });

  it("go around the wheel, any number of times", () => {
    expect(hueColors(360)).toEqual(hueColors(0));
    expect(hueColors(-1)).toEqual(hueColors(359));
  });
});
