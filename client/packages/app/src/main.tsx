import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { I18nProvider } from "./i18n/I18nProvider";
import { applyPalette, applyThemeMode, storedPalette, storedThemeMode } from "./theme/palettes";
import "./styles.css";

applyPalette(storedPalette());
applyThemeMode(storedThemeMode());

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
