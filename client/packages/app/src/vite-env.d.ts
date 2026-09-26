/// <reference types="vite/client" />

interface ImportMetaEnv {
  /**
   * Origin of the Aspen server to talk to, e.g. `https://chat.example.org`. Leave unset to use
   * the page's own origin (same-origin deployment, or the Vite dev proxy).
   */
  readonly VITE_ASPEN_SERVER_URL?: string;
}

/** Injected by the Electron preload script; absent in browsers and Capacitor. */
interface AspenDesktopBridge {
  readonly platform: NodeJS.Platform;
  readonly versions: { electron: string; chrome: string };
}

interface Window {
  readonly aspenDesktop?: AspenDesktopBridge;
}
