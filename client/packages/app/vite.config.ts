import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";
import { defineConfig, loadEnv, type Plugin } from "vite";
import { attributions } from "./attributions/plugin";
import { fallbackFonts } from "./fallbackFonts";

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

/**
 * The desktop app's Content Security Policy, written into its page by `build:desktop` (a page
 * loaded from a file has no headers to carry one; the web client's comes from the server,
 * `api::web_client`). Scripts are the app's own files alone; nothing is loaded from a file but
 * the app's scripts, styles, and fonts, so a `file:` address (another machine's share, on
 * Windows) can never be a picture, a video, or a request; pictures and videos come from
 * deployments over `https:`, or `http:` on this machine for a development one, and requests go
 * to any deployment, over `http:` too, since the person picks the server; frames are video
 * players and plugin views, wherever their deployment is.
 */
const DESKTOP_POLICY = [
  "default-src 'none'",
  "script-src 'self' 'wasm-unsafe-eval'",
  "style-src 'self' 'unsafe-inline'",
  "font-src 'self' data:",
  "img-src data: blob: https: http://localhost:* http://127.0.0.1:* http://*.localhost:*",
  "media-src data: blob: https: http://localhost:* http://127.0.0.1:* http://*.localhost:*",
  "connect-src https: wss: http: ws:",
  "frame-src https: http://localhost:* http://127.0.0.1:* http://*.localhost:*",
  "worker-src 'self' blob:",
  "object-src 'none'",
  "base-uri 'none'",
  "form-action 'none'",
].join("; ");

/**
 * Puts `DESKTOP_POLICY` first in the page's head, and takes out the page's icons, which a
 * window does not show and the policy would refuse to load from a file.
 */
function desktopPolicy(): Plugin {
  return {
    name: "aspen-desktop-policy",
    transformIndexHtml: (html) => ({
      html: html.replace(/\s*<link rel="(?:icon|apple-touch-icon)"[^>]*>/g, ""),
      tags: [
        {
          tag: "meta",
          attrs: { "http-equiv": "Content-Security-Policy", content: DESKTOP_POLICY },
          injectTo: "head-prepend",
        },
      ],
    }),
  };
}

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const proxyTarget = env["VITE_DEV_PROXY_TARGET"] ?? defaultDevProxyTarget;
  return {
    plugins: [
      fallbackFonts(),
      attributions(),
      react(),
      tailwindcss(),
      ...(mode === "desktop" ? [desktopPolicy()] : []),
    ],
    resolve: {
      alias: {
        "@": fileURLToPath(new URL("./src", import.meta.url)),
      },
    },
    // The web build is served from a site root, where absolute asset URLs let a deep link such
    // as `/communities/{id}/channels/{id}` load after a reload. Electron loads the bundle from
    // `file://` and Capacitor from an app-local origin, where only relative URLs resolve, so
    // `build:shell` (Capacitor's) and `build:desktop` (Electron's, which also writes
    // `DESKTOP_POLICY` into the page) pass `--base ./` and route after a `#` instead.
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
