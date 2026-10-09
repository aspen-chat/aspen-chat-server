# About Aspen and Open Source Attributions

About Aspen is a dialog over Settings that shows which deployment and versions the user is on, and their rights under the licence. The attributions page lists every package each part of Aspen ships, with its licence.

## Where it lives

| Part | Code |
| --- | --- |
| About Aspen dialog | `AboutDialog`, `src/features/about/` |
| Attributions page | `/attributions`, `AttributionsScreen` |
| Attributions list | `virtual:attributions` (`Attributions`, `attributionTypes.ts`) |
| Build info | `virtual:build-info` (`BuildInfo`) |
| Vite plugin making both modules | `packages/app/attributions/plugin.ts` |
| What no package manager records | `attributions/native.ts` |
| Licence texts | `attributions/texts/`, `attributions/spdx/` |

## About Aspen

A button beside Sign out at the foot of Settings opens About Aspen. It is a modal over Settings with two planes.

### The first plane: versions

It shows Aspen's icon (`brand/aspen-icon.svg`) and the deployment the user is on. The deployment is named by its display name from `GET /deployment`, or by its address when it has none.

It then lists the versions of everything this copy of Aspen is made of:

| Row | What it shows | Source |
| --- | --- | --- |
| Aspen app | The app's version and the commit it was built from, with "with changes" when the checkout had uncommitted changes. Outside a git checkout only the version shows. | `virtual:build-info` |
| Desktop app | Electron's and Chromium's versions | The preload bridge, `window.aspenDesktop.versions` |
| Phone app | The platform, and the app's version and build | `@capacitor/app`'s `getInfo` |
| Server | The software the server says it runs | `software` in `GET /auth/methods` |
| Protocol | The protocol versions this app speaks and the server's. Each is a single version, or the range from the oldest a side still speaks to the newest. | `CLIENT_PROTOCOL`, and `protocol` in `GET /auth/methods` |

Its link opens the attributions page. That closes Settings, and About Aspen with it.

### The second plane: Your rights

A short summary of the Mozilla Public License 2.0:

- What it lets the user do: run, study, change, and share Aspen, and learn how to get its source from whoever gives them a copy.
- Changes to Aspen's files are shared under it.
- Some parts carry licences of their own; the attributions list them.
- It comes with no warranty.

It links to the licence at mozilla.org and to the source code.

The source link is `BuildInfo.source`. It is `repository` in the client's `package.json`, at the commit the build was made from when the checkout had no changes of its own. **Why:** a fork's build points at the fork.

## Open Source Attributions

### The page

`/attributions` (`AttributionsScreen`) lists every package each part of Aspen ships, grouped by part:

- the app every shell renders
- the desktop app's own binaries
- the phone apps' native code
- the servers

Each package shows its version, licence, website, and licence texts. The texts are drawn only for the package the reader opens, since all of them together are megabytes. A filter narrows every part by name or licence.

Anyone may read it. Signed out, `RootLayout` shows it in place of the sign-in screen, with a link back.

### The list module

The list is the module `virtual:attributions`. The Vite plugin in `packages/app/attributions/plugin.ts` makes it when the app is built or served, so it is never older than the build.

The page imports it dynamically, as a chunk of its own: about 2 MB, 160 kB compressed, fetched when the page opens.

### What each part lists

| Part | What is listed |
| --- | --- |
| The app | The npm packages `@aspen/app` depends on in production, and theirs, walked through `node_modules` as Node resolves them. Workspace packages are Aspen's own, so they are walked through but not listed. Noto Color Emoji, which `scripts/noto_emoji.py` builds, is listed from its directory. |
| The desktop app | Electron (a development dependency, since electron-builder copies it in). Chromium (see below). The capture helper's crates. What the Windows app ships from OBS Studio's release. |
| The phone apps | The npm packages `@aspen/mobile` depends on: Capacitor and its plugins, which carry the native Android and iOS code. |
| The servers | The crates the API server, the voice server, and `aspen-migrate` are built from, on the platforms they are built for. The C and C++ libraries mediasoup's worker builds from its Meson wraps. |

Chromium's notices are too many to carry in the list. They ship as `LICENSES.chromium.html` among the app's resources (`electron-builder.yml`). They open in the system's browser through `window.aspenDesktop.chromiumNotices` (`packages/desktop/src/main/chromiumNotices.ts`).

### Crates

Crates come from `cargo metadata --locked`. The walk follows normal dependencies from the binaries; build scripts and development dependencies ship nothing.

### Licence texts

Each package's licence files are read from its directory:

- npm packages: the top level.
- Crates: three levels down, which reaches the licences of C sources they vendor (`libwebp-sys/vendor/COPYING`).

A package that publishes none gets, in order:

1. Its project's text from `attributions/texts/` (`PACKAGE_TEXTS` in `attributions/native.ts`, for mediasoup's crates).
2. Otherwise, the standard text of each licence its expression names, from `attributions/spdx/`. The page says so.

Identical texts are kept once.

### What no package manager records

`attributions/native.ts` lists these. Each is tied to where its version is decided, so the build stops when that moves:

- mediasoup's subprojects, tied to the wraps of the locked `mediasoup-sys`. Every wrap a shipped build compiles must be listed, at its version.
- The libraries shipped from OBS Studio, tied to `OBS_VERSION` in `fetch-libobs.mjs`.

Their texts in `attributions/texts/` are taken from each project's source at that version. They are kept exactly as published (`.prettierignore`).

### When the list cannot be complete

A build stops when it cannot list everything:

- no `cargo`
- a package with neither a licence file nor a standard text for its licence
- a native library whose version moved

A development server lists what it can, logs what it could not, and the page says the list is short. **Why:** a development server may run where the Rust toolchain is not (Playwright's Docker image).

CI's client and Android jobs install the pinned Rust toolchain for this.

### Not listed yet

- The Maven artifacts of the Android app (AndroidX and Firebase Messaging), which Gradle resolves and the web build cannot see.
- The libraries OBS Studio's own DLLs link statically.

## Build info

`virtual:build-info` (`BuildInfo`) comes from the same plugin. It holds:

- the app's version, from its `package.json`
- the commit, from `git rev-parse`
- whether `git status` shows changes to tracked files
