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
- For the mobile shells: Android Studio and/or Xcode, per Capacitor's requirements. Android builds
  need JDK 21 and the Android SDK (`ANDROID_HOME`, with `platform-tools` on `PATH` for `adb`).

## Getting started

```sh
cd client
pnpm install          # also runs codegen against ../openapi.yaml and ../event_schema.json
pnpm dev              # Vite dev server on http://localhost:5173
pnpm build            # web build (site root); mobile uses `build:shell`, desktop `build:desktop`
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
pnpm --filter @aspen/mobile run:android   # build, sync, and install on a phone (see below)
```

## Running the Android app on a phone

The app bundles a copy of the `build:shell` output rather than loading a dev server, so rebuild
and sync after every change to the client.

Once: turn on Developer options on the phone (tap Settings → About phone → Build number seven
times), turn on USB debugging, plug it in, and accept the prompt. `adb devices` should list it as
`device`, not `unauthorized`.

```sh
pnpm codegen                                  # the build needs the generated types
pnpm --filter @aspen/mobile run:android       # build:shell, cap sync, assemble, install, launch
```

The same steps by hand, as CI runs them:

```sh
pnpm --filter @aspen/app build:shell
(cd packages/mobile && npx cap sync android)
(cd packages/mobile/android && ./gradlew :app:installDebug)
```

`pnpm --filter @aspen/mobile open:android` opens the project in Android Studio instead.

**Reaching a development server.** The app asks for a deployment on first launch. A debug build
may use plain HTTP only to `localhost` and `127.0.0.1`
(`packages/mobile/android/app/src/debug/res/xml/network_security_config.xml`); every other
address needs HTTPS, so a dev server's LAN address is refused. Forward the dev server's port over
USB and enter `http://localhost:5173`:

```sh
adb reverse tcp:5173 tcp:5173                 # again after each reconnect
```

The page is served from `https://localhost`, so anything else it loads over plain HTTP at
another address is mixed content and blocked too: attachments, icons, and uploads fail unless
`[media.s3]`'s `public_endpoint` and `public_base_url` name `localhost` with their ports
forwarded the same way, or the deployment serves them over HTTPS. A call's media is UDP, which
`adb reverse` does not carry, so the phone must reach the voice server over the network.

**Debugging.** `chrome://inspect` in desktop Chrome opens DevTools on the app's web view;
`adb logcat` has the native side. `pnpm --filter @aspen/mobile test:android` runs the JVM tests
and, on the attached phone, the push handler's device tests, which post real notifications.

**Push** works only in a build with a relay in the `aspen_push_relay` string
(`app/src/main/res/values/strings.xml`) and a `google-services.json` from the publisher's
Firebase project; without them the app registers for nothing (see `docs/architecture/push.md`).

## License

MPL-2.0, like the rest of the repository.
