import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";
import { defineConfig, loadEnv } from "vite";
import { attributions } from "./attributions/plugin";
import { fallbackFonts } from "./fallbackFonts";
import { viewFonts } from "./viewFonts";

/**
 * Development proxy target. A deployment is one origin, its API and web client together, so in
 * `vite dev` this origin stands in for it: every path the server owns besides the web client is
 * proxied to the server below, whose `public_url` names this origin
 * (`http://localhost:5173`), so links, passkeys, and the pages the apps open all agree. Point it
 * at a `--no-https` server, or set `VITE_DEV_PROXY_TARGET` to something else.
 */
const defaultDevProxyTarget = "http://127.0.0.1:8000";

/** The paths the server answers itself rather than with the web client (`api::start`). */
const serverPaths = ["/api", "/auth/passkey", "/.well-known/aspen", "/email/unsubscribe"];

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const proxyTarget = env["VITE_DEV_PROXY_TARGET"] ?? defaultDevProxyTarget;
  return {
    plugins: [fallbackFonts(), viewFonts(), attributions(), react(), tailwindcss()],
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
      proxy: Object.fromEntries(
        serverPaths.map((path) => [
          path,
          {
            target: proxyTarget,
            changeOrigin: true,
            // Development servers use a self-signed certificate.
            secure: false,
            ws: path === "/api",
          },
        ]),
      ),
    },
    test: {
      environment: "jsdom",
      include: ["src/**/*.test.{ts,tsx}"],
    },
  };
});
