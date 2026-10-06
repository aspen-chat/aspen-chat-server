# About Aspen and Open Source Attributions

## About Aspen

The last section of Settings (`AboutSection`, `src/features/about/`) shows Aspen's icon
(`brand/aspen-icon.svg`), the deployment the user is on (its display name from
`GET /deployment`, or its address when it has none), and the versions of everything this copy
of Aspen is made of:

- **Aspen app**: the app's version and the commit it was built from, with "with changes" when
  the checkout had uncommitted changes (`virtual:build-info`, below). Outside a git checkout
  only the version shows.
- **Desktop app**: Electron's and Chromium's versions, from the preload bridge
  (`window.aspenDesktop.versions`).
- **Phone app**: the platform, and the app's version and build from `@capacitor/app`'s
  `getInfo`.
- **Server**: the software the server says it runs (`software` in `GET /auth/methods`).
- **Protocol**: the protocol versions this app speaks (`CLIENT_PROTOCOL`) and the server's
  (`protocol` in the same response), each a single version or the range from the oldest a
  side still speaks to the newest.

Its link opens the attributions page and closes Settings. Last comes Your rights: a short
summary of what the GNU General Public License, version 3 or later, lets the user do (run,
study, change, and share Aspen, and have its source from whoever gives them a copy) and that it
comes with no warranty, with links to the license at gnu.org and to the source code. The
source link (`BuildInfo.source`) is `repository` in the client's `package.json`, at the commit
the build was made from when the checkout had no changes of its own, so a fork's build points
at the fork.

## Open Source Attributions

`/attributions` (`AttributionsScreen`) lists every package each part of Aspen ships, by part
(the app every shell renders, the desktop app's own binaries, the phone apps' native code,
and the servers), each with its version, license, website, and its license texts, which are
drawn only for the package the reader opens, since all of them are megabytes. A filter narrows
every part by name or license. Anyone may read it: signed out, `RootLayout` shows it in place
of the sign-in screen, with a link back.

The list is the module `virtual:attributions` (`Attributions`, `attributionTypes.ts`), made by
the Vite plugin in `packages/app/attributions/plugin.ts` when the app is built or served, so it
is never older than the build. The page imports it dynamically: a chunk of its own, about
2 MB, 160 kB compressed, fetched when the page opens.

- **The app**: the npm packages `@aspen/app` depends on, and theirs, in production, walked
  through `node_modules` as Node resolves them. Workspace packages are Aspen's own, so are
  walked through but not listed. Noto Color Emoji, which `scripts/noto_emoji.py` builds, is
  listed from its directory.
- **The desktop app**: Electron, a development dependency since electron-builder copies it in;
  Chromium, whose notices are too many to carry and ship as `LICENSES.chromium.html` among the
  app's resources (`electron-builder.yml`), opened in the system's browser through
  `window.aspenDesktop.chromiumNotices` (`packages/desktop/src/main/chromiumNotices.ts`); the
  capture helper's crates; and what the Windows app ships from OBS Studio's release.
- **The phone apps**: the npm packages `@aspen/mobile` depends on (Capacitor and its plugins,
  which carry the native Android and iOS code).
- **The servers**: the crates the API server, the voice server, and `aspen-migrate` are built
  from on the platforms they are built for, and the C and C++ libraries mediasoup's worker
  builds from its Meson wraps.

Crates come from `cargo metadata --locked`, following normal dependencies from the binaries
(build scripts and development dependencies ship nothing). Each package's license files are
read from its directory: top level for npm packages, and three levels down for crates, which
reaches the licenses of C sources they vendor (`libwebp-sys/vendor/COPYING`). A package that
publishes none is given its project's text from `attributions/texts/` (`PACKAGE_TEXTS` in
`attributions/native.ts`, for mediasoup's crates), or else the standard text of each license
its expression names from `attributions/spdx/`, and the page says so. Identical texts are kept
once.

`attributions/native.ts` lists what no package manager records, each tied to where its version
is decided so the build stops when that moves: mediasoup's subprojects to the wraps of the
locked `mediasoup-sys` (every wrap a shipped build compiles must be listed, at its version),
and the libraries shipped from OBS Studio to `OBS_VERSION` in `fetch-libobs.mjs`. Their texts
in `attributions/texts/` are taken from each project's source at that version, and are kept
exactly as published (`.prettierignore`).

A build stops when it cannot list everything: no `cargo`, a package with neither a license file
nor a standard text for its license, or a native library whose version moved. A development
server lists what it can, logs what it could not, and the page says the list is short, since it
may run where the Rust toolchain is not (Playwright's Docker image). CI's client and Android
jobs install the pinned Rust toolchain for this.

`virtual:build-info` (`BuildInfo`) is the same plugin's: the app's version from its
`package.json`, and the commit from `git rev-parse` with whether `git status` shows changes to
tracked files.

Not listed yet: the Maven artifacts of the Android app (AndroidX and Firebase Messaging, which
Gradle resolves and the web build cannot see), and the libraries OBS Studio's own DLLs link
statically.
