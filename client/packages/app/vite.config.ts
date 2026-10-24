import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { randomUUID } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { Agent } from "node:https";
import { rootCertificates } from "node:tls";
import { fileURLToPath } from "node:url";
import { defineConfig, loadEnv, type Plugin } from "vite";
import { attributions } from "./attributions/plugin";
import { fallbackFonts } from "./fallbackFonts";
import { viewFonts } from "./viewFonts";

/**
 * Development proxy target. A deployment is one origin, its API and web client together, so in
 * `vite dev` this origin stands in for it: every path the server owns besides the web client is
 * proxied to the server below, whose `public_url` names this origin
 * (`http://localhost:5173`, or its `https:` address under `VITE_DEV_HTTPS_CERT`), so links,
 * passkeys, and the pages the apps open all agree. Point it at a `--no-https` server, or set
 * `VITE_DEV_PROXY_TARGET` to something else; an `https:` target's certificate is verified
 * (`proxyAgent`).
 */
const defaultDevProxyTarget = "http://127.0.0.1:8000";

/**
 * The development certificate authority (`scripts/dev_ca.py`), which an `https:` proxy target's
 * certificate is checked against besides the system's authorities, when it has been made.
 */
const devCa = fileURLToPath(new URL("../../../target/dev-ca/ca.pem", import.meta.url));

/**
 * How the proxy reaches an `https:` target: verifying its certificate against the system's
 * authorities and the development one.
 */
function proxyAgent(target: string): Agent | undefined {
  if (!target.startsWith("https:")) return undefined;
  const ca = existsSync(devCa) ? [...rootCertificates, readFileSync(devCa, "utf8")] : undefined;
  return new Agent({ ca, keepAlive: true });
}

/**
 * The dev server's own TLS, from `VITE_DEV_HTTPS_CERT` and `VITE_DEV_HTTPS_KEY` (PEM files, such
 * as `scripts/dev_ca.py issue` writes), for reaching it from another device, where a browser
 * gives plain HTTP none of the APIs the app needs. Without them it serves plain HTTP, which a
 * browser on this machine treats as secure at `localhost`.
 */
function devHttps(env: Record<string, string>): { cert: Buffer; key: Buffer } | undefined {
  const cert = env["VITE_DEV_HTTPS_CERT"];
  const key = env["VITE_DEV_HTTPS_KEY"];
  if (!cert && !key) return undefined;
  if (!cert || !key) throw new Error("give both VITE_DEV_HTTPS_CERT and VITE_DEV_HTTPS_KEY");
  return { cert: readFileSync(cert), key: readFileSync(key) };
}

/** The paths the server answers itself rather than with the web client (`api::start`). */
const serverPaths = [
  "/api",
  "/auth/passkey",
  "/auth/oidc",
  "/.well-known/aspen",
  "/email/unsubscribe",
];

/**
 * The Content Security Policy of the desktop and mobile apps' page, written into it by
 * `build:desktop` and `build:shell` (a page loaded from a file or from the app itself has no
 * headers to carry one; the web client's comes from the server, `api::web_client`). Scripts are
 * the app's own files alone; nothing is loaded from the app's own origin but its scripts,
 * styles, and fonts (and, on a phone, whose origin is the app's bundle, its pictures), so a
 * `file:` address (another machine's share, on Windows) can never be a picture, a video, or a
 * request; pictures, videos, requests, and frames (video players and plugin views) go to
 * deployments over `https:` and `wss:` alone, as the app connects nowhere else
 * (`securelyReached`), and to plain `http:` and `ws:` on this machine only in a development
 * build (the dev server's, or one made with `VITE_ALLOW_LOOPBACK_HTTP=1`), which may reach a
 * deployment there.
 *
 * Capacitor's bridge script on Android is injected ahead of the policy, where an old web view
 * cannot inject it otherwise, and a policy in the page governs only what follows it.
 */
function shellPolicy(shell: "desktop" | "mobile", development: boolean): string {
  const own = shell === "mobile" ? "'self' " : "";
  const loopback = development ? " http://localhost:* http://127.0.0.1:* http://*.localhost:*" : "";
  const loopbackSockets = development
    ? " ws://localhost:* ws://127.0.0.1:* ws://*.localhost:*"
    : "";
  return [
    "default-src 'none'",
    "script-src 'self' 'wasm-unsafe-eval'",
    "style-src 'self' 'unsafe-inline'",
    "font-src 'self' data:",
    `img-src ${own}data: blob: https:${loopback}`,
    `media-src ${own}data: blob: https:${loopback}`,
    `connect-src https: wss:${loopback}${loopbackSockets}`,
    `frame-src https:${loopback}`,
    "worker-src 'self' blob:",
    "object-src 'none'",
    "base-uri 'none'",
    "form-action 'none'",
  ].join("; ");
}

/**
 * Puts the shell's policy (`shellPolicy`) first in the page's head. The desktop app's page also
 * loses its icons, which a window does not show and the policy would refuse to load from a file.
 */
function writtenPolicy(shell: "desktop" | "mobile", development: boolean): Plugin {
  return {
    name: "aspen-shell-policy",
    transformIndexHtml: (html) => ({
      html:
        shell === "desktop"
          ? html.replace(/\s*<link rel="(?:icon|apple-touch-icon)"[^>]*>/g, "")
          : html,
      tags: [
        {
          tag: "meta",
          attrs: {
            "http-equiv": "Content-Security-Policy",
            content: shellPolicy(shell, development),
          },
          injectTo: "head-prepend",
        },
      ],
    }),
  };
}

/**
 * Names each build of the web client: in its page, as `<meta name="aspen-web-client-build">`,
 * which the server serving it reads and gives every event stream (`api::web_client::build`), and
 * in its code, as `__ASPEN_WEB_CLIENT_BUILD__`, so that a page open from before a release can
 * tell it is out of date (`NewRelease`). The development server's page names none: it is not the
 * build the server serves.
 */
function webClientBuild(command: "build" | "serve"): Plugin {
  const build = command === "build" ? randomUUID() : null;
  return {
    name: "aspen-web-client-build",
    config: () => ({ define: { __ASPEN_WEB_CLIENT_BUILD__: JSON.stringify(build) } }),
    transformIndexHtml: () =>
      build === null
        ? []
        : [
            {
              tag: "meta",
              attrs: { name: "aspen-web-client-build", content: build },
              injectTo: "head",
            },
          ],
  };
}

export default defineConfig(({ mode, command }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const proxyTarget = env["VITE_DEV_PROXY_TARGET"] ?? defaultDevProxyTarget;
  const https = command === "serve" ? devHttps(env) : undefined;
  return {
    plugins: [
      fallbackFonts(),
      viewFonts(),
      attributions(),
      webClientBuild(command),
      react(),
      tailwindcss(),
      ...(mode === "desktop" || mode === "mobile"
        ? [writtenPolicy(mode, command === "serve" || env["VITE_ALLOW_LOOPBACK_HTTP"] === "1")]
        : []),
    ],
    resolve: {
      alias: {
        "@": fileURLToPath(new URL("./src", import.meta.url)),
      },
    },
    // The web build is served from a site root, where absolute asset URLs let a deep link such
    // as `/communities/{id}/channels/{id}` load after a reload. Electron loads the bundle from
    // `file://` and Capacitor from an app-local origin, where only relative URLs resolve, so
    // `build:shell` (Capacitor's) and `build:desktop` (Electron's), which also write
    // `shellPolicy` into the page, pass `--base ./` and route after a `#` instead.
    build: {
      outDir: "dist",
      sourcemap: true,
      target: "es2022",
    },
    server: {
      port: 5173,
      strictPort: true,
      ...(https && { https }),
      proxy: Object.fromEntries(
        serverPaths.map((path) => [
          path,
          {
            target: proxyTarget,
            agent: proxyAgent(proxyTarget),
            changeOrigin: true,
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
