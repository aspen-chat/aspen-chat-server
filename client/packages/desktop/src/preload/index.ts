import { contextBridge, ipcRenderer } from "electron";

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
  // Screen sharing's picker: the main process lists the sources, the page shows them and
  // answers with the one chosen.
  displayPicker: {
    onPick: (listener: (request: unknown) => void) => {
      const handler = (_event: unknown, request: unknown) => {
        listener(request);
      };
      ipcRenderer.on("display:pick-source", handler);
      return () => {
        ipcRenderer.off("display:pick-source", handler);
      };
    },
    choose: (choice: unknown) => {
      ipcRenderer.send("display:source-chosen", choice);
    },
  },
  // Passkey ceremonies in the system browser; the main process listens for the browser's return.
  passkeyHandoff: {
    prepare: () => ipcRenderer.invoke("passkey:prepare"),
    open: (id: unknown, url: unknown) => ipcRenderer.invoke("passkey:open", id, url),
    dispose: (id: unknown) => {
      ipcRenderer.send("passkey:dispose", id);
    },
  },
  // `aspen://app/…` links the system handed the app: those that waited, then each as it comes.
  appLinks: {
    ready: () => ipcRenderer.invoke("app-link:ready"),
    onOpen: (listener: (url: string) => void) => {
      const handler = (_event: unknown, url: unknown) => {
        if (typeof url === "string") {
          listener(url);
        }
      };
      ipcRenderer.on("app-link:open", handler);
      return () => {
        ipcRenderer.off("app-link:open", handler);
      };
    },
  },
  // The window's zoom: the factor the main process keeps, and the steps the zoom keys ask for.
  zoom: {
    get: () => ipcRenderer.invoke("zoom:get"),
    set: (factor: unknown) => ipcRenderer.invoke("zoom:set", factor),
    onStep: (listener: (step: number) => void) => {
      const handler = (_event: unknown, step: unknown) => {
        if (step === 1 || step === -1 || step === 0) {
          listener(step);
        }
      };
      ipcRenderer.on("zoom:step", handler);
      return () => {
        ipcRenderer.off("zoom:step", handler);
      };
    },
  },
  // Game capture through libobs; the main process owns the helper that captures and sends it.
  gameCapture: {
    kinds: () => ipcRenderer.invoke("voice:capture-kinds"),
    start: (options: unknown) => ipcRenderer.invoke("voice:capture-start", options),
    startAudio: (audio: unknown) => ipcRenderer.invoke("voice:capture-start-audio", audio),
    stop: () => ipcRenderer.invoke("voice:capture-stop"),
    onEnded: (listener: () => void) => {
      const handler = () => {
        listener();
      };
      ipcRenderer.on("voice:capture-ended", handler);
      return () => {
        ipcRenderer.off("voice:capture-ended", handler);
      };
    },
  },
});
