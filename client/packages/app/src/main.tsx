import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { I18nProvider } from "./i18n/I18nProvider";
import { applyStoredFonts } from "./theme/fonts";
import { applyPalette, applyThemeMode, storedPalette, storedThemeMode } from "./theme/palettes";
// Aspen's own typefaces, upright and italic, each a variable font split by script so a page
// fetches only the scripts it shows (`--font-sans` and `--font-mono` in `styles.css`).
import "@fontsource-variable/inclusive-sans";
import "@fontsource-variable/inclusive-sans/wght-italic.css";
import "@fontsource-variable/noto-sans";
import "@fontsource-variable/noto-sans/wght-italic.css";
// The order of the Noto faces for every other script and for emoji (`fallbackFonts.ts`).
import "./generated/fontStacks.css";
import "@fontsource-variable/intel-one-mono";
import "@fontsource-variable/intel-one-mono/wght-italic.css";
import "./styles.css";

applyPalette(storedPalette());
applyThemeMode(storedThemeMode());
void applyStoredFonts();
// The Noto faces themselves, whose rules are too long to hold up the first paint for; text in
// their scripts shows in the system's fonts until they load.
void import("./generated/fontFaces.css");

const root = document.getElementById("root");
if (root === null) {
  throw new Error("index.html is missing #root");
}

createRoot(root).render(
  <StrictMode>
    <I18nProvider>
      <App />
    </I18nProvider>
  </StrictMode>,
);
