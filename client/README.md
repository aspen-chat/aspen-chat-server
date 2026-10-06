# Aspen client

One TypeScript web application, shipped three ways:

| Package             | What it is                                                                                                                                                        |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `packages/protocol` | Generated types from the server's `openapi.yaml` and `event_schema.json`, plus the HTTP session client and the reconnecting WebSocket event stream built on them. |
| `packages/app`      | The user interface: React 19 and React Aria Components, styled with Tailwind CSS. Built with Vite. Runs in Chrome, Firefox, and Safari.                           |
| `packages/desktop`  | Electron shell (Windows, macOS, Linux). Loads the `app` build; has no UI of its own.                                                                              |
| `packages/mobile`   | Capacitor shell (Android, iOS). Wraps the `app` build in a native project.                                                                                        |

## Prerequisites

- Node 22.12 or newer and pnpm 11 (`corepack enable` picks up the pinned version).
- A Rust toolchain, only if you need to regenerate the server's schema files.
- For the mobile shells: Android Studio and/or Xcode, per Capacitor's requirements.

## Getting started

```sh
cd client
pnpm install          # also runs codegen against ../openapi.yaml and ../event_schema.json
pnpm dev              # Vite dev server on http://localhost:5173
pnpm build            # web build (site root); the desktop and mobile packages use `build:shell`
```

A deployment is one origin, its API and web client together, so the dev server stands in for it:
it proxies every path the server answers itself (`/api`, `/auth/passkey`, `/.well-known/aspen`,
`/email/unsubscribe`) to an Aspen server at `http://127.0.0.1:8000` by default, whose `aspen.toml`
sets `public_url = "http://localhost:5173"` so links and passkeys name the dev server. Start one
with `cargo run -- --no-https --port 8000`; it serves the built web client too, and will not start
until `pnpm build` has made one, though day to day the dev server's is the one you use. Point
the proxy somewhere else with `VITE_DEV_PROXY_TARGET`. The desktop and mobile apps bake in no
server and ask for a deployment on first launch.

## Code generation

```sh
pnpm codegen          # regenerate TypeScript from the existing schema files
pnpm codegen:regen    # ask the server (via cargo) for fresh schema files first
```

Output lands in `packages/protocol/src/generated/` and is gitignored. `pnpm install` and
`pnpm build` both run `codegen`; if the schema files are missing it runs the server's generator
automatically.

## Everyday commands

```sh
pnpm typecheck        # tsc across every package
pnpm lint             # eslint
pnpm test             # vitest across every package
pnpm e2e              # Playwright against Chromium, Firefox, and WebKit (run `pnpm --filter @aspen/app e2e:install` once)
pnpm build            # production build of every package
pnpm dev:desktop      # Electron pointed at the running Vite dev server
pnpm --filter @aspen/desktop package   # installers under packages/desktop/release
pnpm --filter @aspen/mobile add:android && pnpm --filter @aspen/mobile run:android
```

## License

GPL-3.0-or-later, like the rest of the repository.
