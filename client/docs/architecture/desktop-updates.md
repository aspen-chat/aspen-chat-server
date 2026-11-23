# Releasing the desktop app, and its updates

A release of the client is a tag, `client-v<version>`, on a commit of `main`. CI builds the Windows
installer and the Flatpak from it and publishes both to the downloads site (Cloudflare R2 behind
`downloads.aspen-chat.com`) and as a GitHub Release: the installer beside an update feed the
installed Windows app reads to update itself, and the Flatpak as a signed commit of Aspen's Flatpak
repository, which `flatpak update` reads. The other platforms are not released this way yet; the
reasons for the choices below are in `desktop-updates-design-notes.md`.

## Where it lives

| Part                                     | Code                                                                                                                           |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| Versions, tracks, `releases.json`, feeds | `packages/desktop/src/release/releases.ts`                                                                                     |
| Setting and checking the version         | `packages/desktop/src/release/version.ts`                                                                                      |
| Publishing to the bucket                 | `publishWindows.ts` and `publishFlatpak.ts` in `packages/desktop/src/release/`, sharing `storage.ts`                           |
| The Flatpak repository's install files   | `packages/desktop/src/release/flatpakFiles.ts`                                                                                 |
| The bucket and its public address        | `packages/desktop/downloads.json`                                                                                              |
| Who signs, and whose updates are taken   | `packages/desktop/signing.json`                                                                                                |
| The source archives offered              | `packages/desktop/sources.json`                                                                                                |
| The release jobs                         | `publish-windows` and `publish-flatpak` in `.github/workflows/ci.yml`, with `.github/actions/release-tag` and `github-release` |
| The updater                              | `packages/desktop/src/main/updates.ts`, beside `update*.ts` in that folder                                                     |
| The installer's flag                     | `packages/desktop/build/installer.nsh`                                                                                         |
| Settings, and the strip                  | `UpdatesSection` and `UpdateReady` in `packages/app/src/features/updates/`                                                     |

## Versions and tracks

Every package of `client/` carries the client's version. A version is `X.Y.Z` on the stable track or
`X.Y.Z-beta.N` on the beta track, and nothing else is accepted (`parseVersion`), so every version
orders the same way in the publish job, in semver, and in electron-updater.

The beta track always holds the newest build of either track: releasing a stable version newer than
the current beta puts it on both, so beta testers move on to it.

## Making a release

1. `node src/release/version.ts set 0.2.0` in `client/packages/desktop` sets every package's
   version. Commit it to `main`.
2. Tag that commit `client-v0.2.0` and push the tag.
3. CI runs whole on the tag. `desktop-windows` checks first that the tag names the packages' version
   (`version.ts check`), then builds and signs the installer, `Aspen-Setup-0.2.0.exe`, in the
   `windows-signing` environment.
4. Once `desktop-windows`, `client`, and `client-browser` pass, `publish-windows` checks that the
   tag is on `main` (`release-tag`), then:
   1. `publishWindows.ts upload` mirrors the source archives and uploads the installer;
   2. it makes the GitHub Release, a prerelease for a beta, or finds it, and attaches the installer
      (`github-release`);
   3. `publishWindows.ts announce` writes the track's feed and `releases.json`.
5. Once `desktop-flatpak` and the same checks pass, `publish-flatpak` does the same for the Flatpak
   (see [The Flatpak repository](#the-flatpak-repository)). The two publish jobs share a concurrency
   group, so one runs after the other, each reading the `releases.json` the other wrote.

Both run in the `r2-publishing` environment, which only `client-v*` tags may use. It holds:

- `R2_ENDPOINT` (a variable): the account's S3 endpoint,
  `https://<account>.r2.cloudflarestorage.com`, without the bucket the dashboard shows after it
  (`downloads.json` names the bucket)
- `R2_ACCESS_KEY_ID` and `R2_SECRET_ACCESS_KEY` (secrets): an R2 token for that bucket alone
- `FLATPAK_GPG_KEY` (a secret): the Flatpak repository's signing subkey alone, armoured, with no
  passphrase

The `windows-signing` environment must admit `client-v*` tags as well as `main`.

## The downloads site

The bucket `downloads.json` names, served at its `url`:

| Path                                                | What                                                                                        | Cached    |
| --------------------------------------------------- | ------------------------------------------------------------------------------------------- | --------- |
| `windows/x64/<version>/Aspen-Setup-<version>.exe`   | Each installer, written once                                                                | a year    |
| `windows/x64/latest.yml`, `windows/x64/beta.yml`    | The stable and beta feeds electron-updater reads                                            | 5 minutes |
| `releases.json`                                     | The newest build of each track, by platform (`windows-x64`), with its address and checksums | 5 minutes |
| `sources/<archive>`                                 | The source archives `sources.json` names, written once                                      | a year    |
| `linux/x64/<version>/Aspen-<version>.flatpak`       | Each signed Flatpak bundle, written once                                                    | a year    |
| `flatpak/repo/`                                     | The Flatpak repository: objects and deltas a year, its refs and summary not at all          | as said   |
| `flatpak/aspen.flatpakrepo`                         | Adds the repository as a remote, `aspen`                                                    | not       |
| `flatpak/aspen.flatpakref`, `aspen-beta.flatpakref` | Installs the app's `stable` or `beta` branch in one step                                    | not       |

`releases.json` is what the site's download page reads, and what the publish jobs read back to
decide the next release. Its builds are keyed `windows-x64` and `linux-x64-flatpak` (the bundle).
Each build in it has `version`, `url`, `sha256` (hexadecimal), `sha512` (base64, as the feeds carry
it), `size`, and `released`.

### What publishing refuses

- A version no newer than the head of its own track: its feed would offer an older build, and a
  version already released would be released twice.
- An installer whose name is not that of the package's version.
- An installer at a path already holding another build. The installer is uploaded with its SHA-256
  as metadata; one with the same SHA-256 is left alone, so a publish that stopped part way (a re-run
  of `publish-windows` alone, with the same artifact) carries on.
- `announce` before `upload`: the installer and every source archive must already be there.
- A source archive whose SHA-256 differs from the one `sources.json` pins, or a commit fetched as
  another than the one it pins.

Files that never change are written first and the feeds and `releases.json` last, so nobody is
pointed at a file not there yet. The job's concurrency group runs one publish at a time, since each
reads what the last wrote.

### The source archives

The installer ships OBS Studio's own files (`screen-sharing-and-game-capture/windows-capture.md`),
which include copyleft libraries: OBS Studio and x264 (GPL), FFmpeg (GPL, as obs-deps builds it),
LAME and pthreads-win32 (LGPL), and libdatachannel, libjuice, and SRT (MPL). Their licences oblige
whoever distributes them to offer their source, so the publish job mirrors it beside the installer,
and the Open Source Attributions page links each library to its archives (`about.md`).

Electron ships FFmpeg too, as Chromium's copy of it, built as a library of its own (`ffmpeg.dll`,
`libffmpeg.so`, `libffmpeg.dylib`) under the LGPL, so its source is offered the same way: Chromium's
FFmpeg at the revision Chromium's `DEPS` pins for the Chromium version Electron's `DEPS` names, and
Electron's own source at its tag, which holds its patch to FFmpeg and the arguments its release is
built with (`build/args/release.gn`). Every desktop build ships it, not only the Windows one.

`sources.json` holds the archives in two groups, each of the version it was read for:

- **`obs`**, read from the OBS Studio release `fetch-libobs.mjs` fetches: its `CMakePresets.json`
  names the obs-deps tag it was built with, and that tag's scripts name each library's revision.
  `fetch-libobs.mjs` refuses to fetch another OBS Studio release until the group is read again for
  it.
- **`electron`**, read from the Electron installed, as above. The app's build stops when another
  Electron is installed (`attributions/plugin.ts`).

An archive is either downloaded from where its project publishes it and checked against a pinned
SHA-256, or, where the only archive offered is made afresh on each request and so differs every time
(Chromium's FFmpeg, from gitiles), made by the publish job from a pinned commit (`git` and
`commit`), which git checks the fetched objects against. Either is uploaded once and kept.

### The Flatpak repository

`flatpak/repo/` is one OSTree repository holding the app's `stable` and `beta` branches
(`app/com.aspen_chat.client/x86_64/<track>`), signed with the key whose public half is
`packages/desktop/flatpak/aspen-packages.asc`, which the `.flatpakrepo` and `.flatpakref` files
carry. A Flatpak release, `publishFlatpak.ts`:

1. `upload`:
   1. reads the repository from the bucket into a work folder;
   2. imports `desktop-flatpak`'s unsigned bundle and commits its app to the branch of each track
      the version heads, signed, with the branch's head before it as its parent and the release
      commit's time as its timestamp;
   3. has Flatpak make the static deltas, the AppStream data software centres read, and a signed
      summary, and prune each branch to its 10 newest commits (`HISTORY`) and what they hold;
   4. makes a signed bundle of the new commit that names the repository and its key, so a bundle
      installed by hand updates from the repository;
   5. uploads the source archives, the repository's new objects and deltas, and the bundle.
2. The GitHub Release gets the signed bundle.
3. `announce` writes the repository's refs, delta indexes, and summaries (the summary and its index
   last), the `.flatpakrepo` and `.flatpakref` files, and `releases.json`, then removes from the
   bucket what the repository no longer holds.

The commit's timestamp makes it the same commit when `upload` runs again from the same published
state, so a re-run of `publish-flatpak` alone finds its bundle already uploaded (by the commit
recorded in the bundle's metadata) and carries on. A key that is not the published one is refused
before anything is written, since people's machines would refuse what it signs.

The runtime is not in the repository: the files name Flathub's for it
(`https://dl.flathub.org/repo/flathub.flatpakrepo`), which `flatpak install` adds where it is
missing.

### Trying an update without releasing one

`publishWindows.ts --prefix test/` publishes beneath `test/` in the bucket, with feeds and a
`releases.json` of its own. An installed build started with `ASPEN_UPDATE_FEED` set to
`<downloads url>/test/windows/x64` reads that feed instead; its updates must still be signed by a
publisher `signing.json` takes updates from. `ASPEN_DOWNLOADS_URL` and `ASPEN_DOWNLOADS_BUCKET`
point the script at a stand-in for R2.

## The updater

Only the signed Windows build updates itself: `electron-builder.signed.mjs` marks its packaged
`package.json` (`aspen.updates`), and every other build, and every other platform, is given no
updater at all.

- **What it reads.** The feed of the track the person follows, through electron-updater's
  `NsisUpdater`. Its configuration is written by the app into the profile (`app-update.yml`) from
  `downloads.json` and `signing.json`, rather than read from the file electron-builder packages.
- **When.** Thirty seconds after starting, then every four hours.
- **What it downloads.** A newer version than the one installed, never an older one, whole (the
  feeds carry no blockmap). Someone following stable never installs a beta, even one downloaded
  while they followed beta.
- **What it checks.** That the download is validly signed by a publisher in `signing.json`'s
  `updatesFrom`, through Windows' own Authenticode check (`updateSignature.ts`). Every way that
  check can fail refuses the update.
- **When it installs.** As the app quits, or at once from Restart now, which the strip above the app
  (`UpdateReady`) and Settings offer while an update waits.

### Turning it off

Settings offers Releases, Betas and releases, and Nothing, kept in the profile (`updates.json`). A
person who has chosen nothing follows the track the installed build came from.

Two switches outside the app win over that choice, and Settings then says which instead of offering
the choice (`updatePolicy.ts`):

- **An administrator**: `DisableUpdates` under `HKLM\Software\Policies\Aspen`, for every account on
  the machine.
- **The installer**: `Aspen-Setup-<version>.exe /DISABLEUPDATES` writes `DisableUpdates` under
  `HKCU\Software\Aspen`, for the account it installs for. Uninstalling removes it; an update does
  not.

A value of the DWORD 0 leaves updating on, and any other value turns it off. A policy that cannot be
read (PowerShell will not run, or answers something else) is taken as off.

With updating off, no updater is made and nothing is fetched from the downloads site.

## Signing

`signing.json`'s `windows.signer` is the certificate `electron-builder.signed.mjs` signs with (its
common name), and `updatesFrom` the publishers an installed app takes updates from, which must
include the signer. CI checks that the installer and the app carry the signer's signature.

To move signing to another publisher, add it to `updatesFrom`, release that build, and only then
change `signer`: every installed copy must already accept the new publisher before a build signed by
it is offered.
