# The Flatpak

The desktop app is packaged as a Flatpak for Linux, alongside the other Linux packages. It carries the same unpacked build they do; nothing is compiled for it.

## Where it lives

| Part                                                        | Code                                                |
| ----------------------------------------------------------- | --------------------------------------------------- |
| Manifest: runtime, permissions, and what is installed where | `packages/desktop/flatpak/org.aspenchat.client.yml` |
| Launcher                                                    | `flatpak/aspen.sh`                                  |
| Desktop entry                                               | `flatpak/org.aspenchat.client.desktop`              |
| What software centres show                                  | `flatpak/org.aspenchat.client.metainfo.xml.in`      |
| Build script                                                | `packages/desktop/scripts/flatpak.mjs`              |
| The window naming its desktop entry                         | `src/main/index.ts`                                 |

## Building it

`pnpm --filter @aspen/desktop package:flatpak`, on Linux. It runs `package:dir`, which leaves the unpacked app in `release/linux-unpacked`, then `scripts/flatpak.mjs`.

The script needs:

- `flatpak-builder` on the `PATH`, or the `org.flatpak.Builder` Flatpak.
- The runtime, SDK, and base app the manifest names, installed from Flathub:

  ```sh
  flatpak install flathub org.freedesktop.Platform//25.08 org.freedesktop.Sdk//25.08 org.electronjs.Electron2.BaseApp//25.08
  ```

It writes, under `release/flatpak/`:

| Output                    | What it is                                                                                     |
| ------------------------- | ---------------------------------------------------------------------------------------------- |
| `repo/`                   | A Flatpak repository holding this build alone. Flatpak installs and updates from repositories. |
| `Aspen-<version>.flatpak` | The same build as one file, to install by hand (`flatpak install ./Aspen-<version>.flatpak`).  |

`--branch` chooses the branch the build is committed to: `stable` (the default) or `beta`.

The script fills the version from `package.json` and the build's date into the metainfo's release.

### Signing

`ASPEN_FLATPAK_GPG_KEY` names the GnuPG key that signs the repository's commits and summary, and the bundle. `ASPEN_FLATPAK_GPG_HOMEDIR` names its keyring when it is not the default one.

Without a key the build is unsigned. **Why it matters:** Flatpak installs whatever a remote's summary names, so a repository people add as a remote must be signed, and they are given the public key with it. An unsigned build is for installing by hand.

## The runtime and the base app

The app runs on `org.freedesktop.Platform`, which people's machines get from Flathub. The bundle names Flathub as the place to find it.

`org.electronjs.Electron2.BaseApp` is built into the app. It brings zypak. **Why:** Chromium sandboxes its processes with a setuid helper or with user namespaces, and a Flatpak allows neither. zypak has Flatpak make those sandboxes instead. The launcher starts the app through `zypak-wrapper`, and the manifest removes `chrome-sandbox`.

The binaries are not stripped (`build-options` in the manifest), so they are the ones electron-builder made.

## Permissions

| Permission                                                 | What needs it                                                                                                                                        |
| ---------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--share=network`                                          | Everything. The passkey hand-off's loopback listener too: the system browser reaches it because the app shares the machine's network.                |
| `--socket=wayland`, `--socket=fallback-x11`, `--share=ipc` | The window. X11 is offered only where there is no Wayland.                                                                                           |
| `--socket=pulseaudio`                                      | The microphone and speakers in calls.                                                                                                                |
| `--device=all`                                             | The camera, for which Flatpak has no narrower permission, and the GPU.                                                                               |
| `--filesystem=xdg-run/pipewire-0`                          | The capture helper, which lists the applications playing sound and takes one's through PipeWire. Without it the helper reports no application audio. |
| `--talk-name=org.freedesktop.Notifications`                | Notifications.                                                                                                                                       |

The app has no access to the user's files. Choosing a file to attach, and saving one, go through the desktop's file chooser portal.

## The desktop entry

The entry registers the `aspen://` scheme (`MimeType`), so links open the app.

Inside a Flatpak the shell calls `app.setDesktopName` with `$FLATPAK_ID.desktop`. **Why:** a desktop gives a window its entry's icon, and groups it with its other windows, only when the window names that entry. Electron otherwise names one after the package.
