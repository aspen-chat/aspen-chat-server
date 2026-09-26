import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";
import { defineConfig, loadEnv } from "vite";

/**
 * Development proxy target. When `VITE_ASPEN_SERVER_URL` is unset the app talks to its own
 * origin, and in `vite dev` that origin proxies `/api` to the server below, so the browser
 * needs neither CORS nor a trusted certificate. Point it at a `--no-https` server, or set
 * `VITE_DEV_PROXY_TARGET` to something else.
 */
const defaultDevProxyTarget = "http://127.0.0.1:8000";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const proxyTarget = env["VITE_DEV_PROXY_TARGET"] ?? defaultDevProxyTarget;
  return {
    plugins: [react(), tailwindcss()],
    resolve: {
      alias: {
        "@": fileURLToPath(new URL("./src", import.meta.url)),
      },
    },
    // The web build is served from a site root, where absolute asset URLs let a deep link such
    // as `/communities/{id}/channels/{id}` load after a reload. Electron loads the bundle from
    // `file://` and Capacitor from an app-local origin, where only relative URLs resolve, so
    // `build:shell` passes `--base ./` for those two and they route after a `#` instead.
    build: {
      outDir: "dist",
      sourcemap: true,
      target: "es2022",
    },
    server: {
      port: 5173,
      strictPort: true,
      proxy: {
        "/api": {
          target: proxyTarget,
          changeOrigin: true,
          // Development servers use a self-signed certificate.
          secure: false,
          ws: true,
        },
      },
    },
    test: {
      environment: "jsdom",
      include: ["src/**/*.test.{ts,tsx}"],
    },
  };
});
