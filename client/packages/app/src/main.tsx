import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { FollowDocumentTitle } from "./features/layout/documentTitle";
import { TitleBar } from "./features/layout/TitleBar";
import { reloadForNewRelease } from "./features/layout/releaseReload";
import { I18nProvider } from "./i18n/I18nProvider";
import { applyStoredFonts } from "./theme/fonts";
import {
  applyContrastMode,
  applyPalette,
  applyThemeMode,
  storedContrastMode,
  storedPalette,
  storedThemeMode,
} from "./theme/palettes";
import { followSystemTextSize } from "./theme/systemTextSize";
import { followTitleBar } from "./theme/titleBar";
import { followZoom } from "./theme/zoom";
// Aspen's own typefaces.
import "./faces.css";
// The order of the Noto faces for every other script and for emoji (`fallbackFonts.ts`).
import "./generated/fontStacks.css";
import "./styles.css";

applyPalette(storedPalette());
applyThemeMode(storedThemeMode());
applyContrastMode(storedContrastMode());
void applyStoredFonts();
followZoom();
followSystemTextSize();
followTitleBar();
// The Noto faces themselves, whose rules are too long to hold up the first paint for; text in
// their scripts shows in the system's fonts until they load.
void import("./generated/fontFaces.css");

// A part of the app loaded on demand (a language's highlighting, the QR code reader) that the
// server no longer has: this page is of a release since replaced. Reloading brings the one
// served now; a failure moments after a reload is left to show as itself.
window.addEventListener("vite:preloadError", (event) => {
  if (reloadForNewRelease()) {
    event.preventDefault();
  }
});

const root = document.getElementById("root");
if (root === null) {
  throw new Error("index.html is missing #root");
}

createRoot(root).render(
  <StrictMode>
    <I18nProvider>
      <FollowDocumentTitle />
      <TitleBar />
      <App />
    </I18nProvider>
  </StrictMode>,
);
