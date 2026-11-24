import { defineConfig, externalizeDepsPlugin } from "electron-vite";

/**
 * Builds only the main and preload processes. The renderer is the `@aspen/app` package: in
 * development the window loads the Vite dev server, in production it loads the app's built
 * files, which electron-builder copies into the bundle as an extra resource.
 */
export default defineConfig({
  main: {
    // The package ships no `node_modules`, every dependency being a development one, so what the
    // main process loads besides Electron and Node is bundled into it.
    plugins: [externalizeDepsPlugin({ exclude: ["electron-updater", "builder-util-runtime"] })],
    build: {
      rollupOptions: {
        input: "src/main/index.ts",
      },
    },
  },
  preload: {
    plugins: [externalizeDepsPlugin()],
    build: {
      rollupOptions: {
        input: "src/preload/index.ts",
        output: {
          // Sandboxed preload scripts must be CommonJS.
          format: "cjs",
          entryFileNames: "[name].cjs",
        },
      },
    },
  },
});
