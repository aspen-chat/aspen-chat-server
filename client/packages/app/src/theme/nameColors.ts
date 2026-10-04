/**
 * The colours names are drawn in. A role carries only a hue, 0 to 359 around HSV's wheel (0
 * red, 120 green, 240 blue); the saturation and lightness are chosen here, so that every hue
 * reads in every palette and both schemes. HSV's own saturation and value cannot do that: at
 * one setting a yellow is far lighter than a blue, and no setting makes both readable. So each
 * hue is drawn in OKLCH, whose lightness is perceptual, at the OKLCH hue of HSV's fully saturated
 * colour of that hue, so the colour is the one the hue names. It starts from one lightness and
 * the chroma it aims for, the chroma lowered where sRGB cannot show it, and then darkens (in
 * light mode) or lightens (in dark mode) until it reaches `MIN_CONTRAST` against the hardest
 * ground a name sits on. Those grounds are the palettes' surfaces and `accent-soft`, behind
 * mentions; the test checks every hue against each of them in `styles.css`.
 */

/** WCAG's minimum contrast for text, with a little room for rounding. */
export const MIN_CONTRAST = 4.6;

/** The relative luminance of the darkest light-scheme ground among the palettes (Dusk's hover). */
const DARKEST_LIGHT_GROUND = luminanceOf("#e3e0f0");
/** The relative luminance of the lightest dark-scheme ground (Aspen's dark `accent-soft`). */
const LIGHTEST_DARK_GROUND = luminanceOf("#064e3b");

interface Scheme {
  /** The OKLCH lightness each hue starts from. */
  lightness: number;
  /** The chroma each hue aims for. */
  chroma: number;
  /** Which way lightness moves until the colour reads: down on light grounds, up on dark. */
  step: number;
  /** Whether a colour of this luminance reads on this scheme's hardest ground. */
  reads: (luminance: number) => boolean;
}

const LIGHT: Scheme = {
  lightness: 0.56,
  chroma: 0.16,
  step: -0.005,
  reads: (luminance) => contrast(luminance, DARKEST_LIGHT_GROUND) >= MIN_CONTRAST,
};

const DARK: Scheme = {
  lightness: 0.78,
  chroma: 0.13,
  step: 0.005,
  reads: (luminance) => contrast(luminance, LIGHTEST_DARK_GROUND) >= MIN_CONTRAST,
};

/** The two colours of one hue, as `#rrggbb`. */
export interface HueColors {
  light: string;
  dark: string;
}

const made = new Map<number, HueColors>();

/** The colours a name of HSV hue `hue` is drawn in, on light grounds and on dark. */
export function hueColors(hue: number): HueColors {
  const key = ((Math.round(hue) % 360) + 360) % 360;
  let colors = made.get(key);
  if (colors === undefined) {
    const angle = oklchHueOfHsv(key);
    colors = { light: colorFor(angle, LIGHT), dark: colorFor(angle, DARK) };
    made.set(key, colors);
  }
  return colors;
}

/** A CSS colour for `hue` that follows the scheme shown, through `light-dark()`. */
export function hueColor(hue: number): string {
  const { light, dark } = hueColors(hue);
  return `light-dark(${light}, ${dark})`;
}

function colorFor(hue: number, scheme: Scheme): string {
  let lightness = scheme.lightness;
  for (;;) {
    const hex = toHex(oklchToLinear(lightness, gamutChroma(lightness, hue, scheme.chroma), hue));
    if (scheme.reads(luminanceOf(hex)) || lightness <= 0 || lightness >= 1) {
      return hex;
    }
    lightness += scheme.step;
  }
}

/** The most chroma up to `wanted` that sRGB can show at this lightness and hue. */
function gamutChroma(lightness: number, hue: number, wanted: number): number {
  if (inGamut(oklchToLinear(lightness, wanted, hue))) {
    return wanted;
  }
  let low = 0;
  let high = wanted;
  for (let i = 0; i < 20; i++) {
    const middle = (low + high) / 2;
    if (inGamut(oklchToLinear(lightness, middle, hue))) {
      low = middle;
    } else {
      high = middle;
    }
  }
  return low;
}

type Linear = readonly [number, number, number];

/** The OKLCH hue, in degrees, of HSV's colour of hue `hue` at full saturation and value. */
function oklchHueOfHsv(hue: number): number {
  const channel = (n: number) => {
    const k = (n + hue / 60) % 6;
    return 1 - Math.max(0, Math.min(k, 4 - k, 1));
  };
  const [r, g, b] = [channel(5), channel(3), channel(1)].map(decode) as [number, number, number];
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return ((Math.atan2(bb, a) * 180) / Math.PI + 360) % 360;
}

/** An sRGB channel, 0 to 1, made linear. */
function decode(encoded: number): number {
  return encoded <= 0.04045 ? encoded / 12.92 : ((encoded + 0.055) / 1.055) ** 2.4;
}

/** OKLCH to linear sRGB, by Björn Ottosson's OKLab matrices. */
function oklchToLinear(lightness: number, chroma: number, hue: number): Linear {
  const radians = (hue * Math.PI) / 180;
  const a = chroma * Math.cos(radians);
  const b = chroma * Math.sin(radians);
  const l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

function inGamut(color: Linear): boolean {
  return color.every((channel) => channel >= -1e-6 && channel <= 1 + 1e-6);
}

function toHex(color: Linear): string {
  return `#${color
    .map((channel) => {
      const clamped = Math.min(1, Math.max(0, channel));
      const encoded = clamped <= 0.0031308 ? 12.92 * clamped : 1.055 * clamped ** (1 / 2.4) - 0.055;
      return Math.round(encoded * 255)
        .toString(16)
        .padStart(2, "0");
    })
    .join("")}`;
}

/** The WCAG relative luminance of a `#rrggbb` colour. */
export function luminanceOf(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((at) =>
    decode(Number.parseInt(hex.slice(at, at + 2), 16) / 255),
  ) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** The WCAG contrast ratio of two relative luminances. */
export function contrast(a: number, b: number): number {
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}
