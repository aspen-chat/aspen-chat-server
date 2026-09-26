import { contextBridge } from "electron";

/**
 * The only surface the renderer sees from the host. Keep it small and data-only; every entry
 * here is reachable by any script running in the page. The shape is declared for the app in
 * `packages/app/src/vite-env.d.ts` as `AspenDesktopBridge`.
 */
contextBridge.exposeInMainWorld("aspenDesktop", {
  platform: process.platform,
  versions: {
    electron: process.versions.electron,
    chrome: process.versions.chrome,
  },
});
