# Distributing Aspen

How each build of Aspen reaches people and stays up to date. Most of this is a plan: the **Built**
column of each table, and [What exists](#what-exists), say what is in the tree today. Change this
file when a decision changes or a part of it is built.

Aspen is published by **Bitcrafters LLC**, which signs the builds, holds the repository signing key,
owns the domain the files are served from, and runs the push relay (`spec/push.md`).

## The rule

Each platform updates Aspen through its own mechanism wherever it has one. Where Aspen updates
itself, people can turn that off, and once it is off the app never contacts the update server.

## Channels

Every channel has a **stable** and a **beta** track. A beta is a release candidate of the next
stable version, published to the same places under the names below.

### Linux desktop

The Linux builds never update themselves.

| Build                  | How it updates                                                                                                                                                                  | Beta              | Built                           |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------- | ------------------------------- |
| `.deb` and `.rpm`      | Installing one adds Aspen's signed APT or DNF repository, as Chrome's packages do, unless `/etc/default/aspen` says not to; the operating system's updates then carry new ones. | a `beta` suite    | `.deb` only, with no repository |
| `aspen-bin` on the AUR | The AUR helper, from a package Aspen maintains.                                                                                                                                 | `aspen-beta-bin`  | no                              |
| Flatpak                | `flatpak update`, from Aspen's own GPG-signed repository, added with a `.flatpakrepo` file.                                                                                     | the `beta` branch | the package (`flatpak.md`)      |
| `.tar.gz`              | It does not; a person downloads the next one.                                                                                                                                   | a beta archive    | no                              |

The Flatpak's runtime comes from Flathub; Aspen itself is not published there.

### Windows

| Build                 | How it updates                                              | Beta               | Built                         |
| --------------------- | ----------------------------------------------------------- | ------------------ | ----------------------------- |
| Signed NSIS installer | [Aspen's updater](#aspens-updater), from Aspen's own site.  | the `beta` feed    | the installer and its signing |
| winget                | `winget upgrade`; the listing points at the same installer. | a separate listing | no                            |

An MSI is made only if businesses ask for one.

### macOS

| Build                      | How it updates                                                                  | Beta            | Built                        |
| -------------------------- | ------------------------------------------------------------------------------- | --------------- | ---------------------------- |
| Developer ID notarized dmg | [Aspen's updater](#aspens-updater).                                             | the `beta` feed | signed, notarized dmgs in CI |
| Homebrew cask              | The app itself: the cask is marked `auto_updates`, so Homebrew leaves it alone. | `aspen@beta`    | no                           |

electron-updater installs from a `zip` on macOS, so the mac target gains one beside each dmg. A Mac
App Store build may be added later; none is planned now.

### Android

| Build                              | How it updates                                                                                              | Beta                        | Built                                       |
| ---------------------------------- | ----------------------------------------------------------------------------------------------------------- | --------------------------- | ------------------------------------------- |
| Google Play                        | Play. Push through Firebase and Bitcrafters' relay.                                                         | the open testing track      | an unsigned APK in CI (`android-builds.md`) |
| F-Droid, without Google's services | F-Droid, from its main repository and from Aspen's own on R2. Push through UnifiedPush. Built reproducibly. | Aspen's own repository only | no                                          |

The two need build flavours, `play` and `foss`, the second without Firebase. Release signing keys
come once Aspen's Google Play Console account is set up.

### iOS

The App Store, with TestFlight for the beta, once what [The stores](#the-stores) lists is built.

### Servers

| What                                                     | Where                                                  | Built                               |
| -------------------------------------------------------- | ------------------------------------------------------ | ----------------------------------- |
| `aspen-api` and `aspen-voice`, for x86-64 and 64-bit ARM | GHCR, tagged by version and `beta`, signed with cosign | the images, unsigned (`images.yml`) |

Servers never update themselves. Each server's dashboard says when a newer version is released, on
by default, from a `releases.json` on Aspen's own domain, so checking reveals nothing to GitHub.

## Aspen's updater

The Windows installer and the macOS dmg update themselves with `electron-updater`, from static feeds
Aspen publishes (`latest.yml` and `beta.yml`, and their mac counterparts).

- It downloads a new version in the background, installs it when the app quits, and tells the person
  one is waiting.
- Settings chooses stable or beta, and can turn updating off.
- Administrators can turn it off for everyone on a machine: `HKLM\Software\Policies\Aspen` on
  Windows, and a configuration profile for `com.aspen-chat.client` on macOS. The installer takes a
  flag that does the same.
- A feed carries a staged rollout percentage, so a release reaches everyone over days rather than at
  once.
- A check sends only what the request itself shows: the address it comes from and the version
  asking.

## Hosting

Installers, update feeds, `releases.json`, and the APT, DNF, Flatpak, and F-Droid repositories are
static files in Cloudflare R2, on Bitcrafters' domain behind its CDN. GitHub Releases mirrors the
installers. Feeds carry no blockmaps: R2 charges nothing for what it serves, so differential
downloads save nobody money.

Publishing writes files that never change (installers, packages, repository objects) first and the
metadata that names them (feeds, repository indexes) last, so nobody is pointed at a file not there
yet, and one publish runs at a time.

## Signing

| What                               | Key                                                                          |
| ---------------------------------- | ---------------------------------------------------------------------------- |
| APT, DNF, and Flatpak repositories | Bitcrafters' GPG key, kept offline; CI signs with a subkey given as a secret |
| Windows installer                  | Azure Trusted Signing (`electron-builder.signed.yml`)                        |
| macOS dmg and App Store builds     | Apple Developer ID and distribution certificates (`apple-builds.md`)         |
| Android                            | Play app signing for Play; Aspen's own key for F-Droid                       |
| Server images                      | cosign                                                                       |

The Windows and Apple builds are signed today by Jacob Kiesel as an individual (publisher
`Jacob Kiesel`, Apple team `L2GS25U79G`), and move to Bitcrafters:

- **Windows**: `publisherName` takes a list. One release naming both publishers, signed by the old,
  lets every installed copy accept the next, signed by Bitcrafters.
- **macOS**: a new Apple team changes the Team ID, and an installed app cannot update to one signed
  by another team; people would reinstall once. Moving before the first public dmg avoids that.

The libobs files shipped on Windows are the OBS Project's own signed releases and are never
re-signed (`scripts/check-libobs.mjs`).

## The stores

What publishing in the App Store and Google Play rests on. Google Play also waits on Aspen's Play
Console account, which its signing keys come with.

- **18+ only.** The mobile apps ask the operating system for the person's age range (Declared Age
  Range on iOS, Play Age Signals on Android). Where the device reports someone under 18 in a
  jurisdiction that requires the check, the app shows only this, and nothing else of Aspen: "Your
  device has indicated you are not old enough to use this app. We do not store your age in any
  location. This determination is made by your operating system and legal jurisdiction." The age is
  never stored, in the app or anywhere else. A device that reports no age range lets the person in.
- **The app's privacy policy** is Bitcrafters', published on its domain and linked from the store
  listings. It makes the promise Mastodon's iOS app policy makes, in words of its own: the app
  collects nothing, and the deployments it connects to are covered by their own policies:

  > The Aspen app, published by Bitcrafters LLC, does not collect or process any personal
  > information about the people who use it.
  >
  > The app connects to Aspen deployments run by others, which may collect and process personal
  > information. This policy does not cover them: each deployment publishes a Privacy Policy of its
  > own, which you can read in the app or on that deployment's website.

  The app itself has no terms of use; each deployment's are its own.

- **Each deployment's own Privacy Policy and Terms of Use**, written by its administrators in the
  dashboard and linked from the sign-in page's footer. Aspen gives no default for either; a
  deployment without one shows "This deployment has not published a Privacy Policy." or "This
  deployment has not published Terms of Use." where it would be.

Both stores also need what Aspen lacks: deleting one's account in the app, a contact link, a
deployment and account for the stores' reviewers, and calls that keep running in the background.

## Not used

- **Flathub**: its policies do not fit how Aspen is built and released.
- **MSIX and the Microsoft Store**: people do not value them, and they restrict the app too much.
- **AppImage**: it needs FUSE 2, and Ubuntu's restriction on user namespaces breaks it.
- **update.electronjs.org**: it needs Squirrel.Windows and GitHub Releases, has no staged rollout,
  and puts a third party in the update path.
- **A Linux build that updates itself.**

## What exists

- The `.deb`, the Flatpak (`client/docs/architecture/flatpak.md`), the Windows installer and its
  signing, and the macOS dmgs with their signing and notarization, all built in CI. electron-builder
  also builds an AppImage, which is not published.
- The server images, published to GHCR on every push to `main` and every tag (`images.yml`).
- Android `versionCode` from the CI run number (`android-builds.md`).

Nothing is published to R2, no repository or feed exists, the desktop app has no updater, the
dashboard has no update notice, and Android has neither flavour nor release signing.
