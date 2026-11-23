/**
 * What `plugin.ts` cannot find in a package manager's records: the C and C++ libraries built or
 * shipped beside the packages it walks, and license texts for packages that carry none. Each
 * text named here is a file in `texts/` (taken from the library's own source at the version
 * shipped) or a standard text in `spdx/`.
 *
 * Every entry is tied to where its version is decided, and the build stops when that moves, so
 * an update cannot ship with this list describing the version before it: what the Windows
 * desktop app ships from OBS Studio's release, to `OBS_VERSION` in
 * `packages/desktop/scripts/fetch-libobs.mjs`, what the desktop app ships inside Electron, to
 * the Electron installed, and what the Flatpak carries from its base app, to the base app's
 * commit `packages/desktop/sources.json` pins.
 */

import type { ComponentId } from "../src/features/about/attributionTypes";

export interface NativeLibrary {
  readonly component: ComponentId;
  readonly name: string;
  readonly license: string;
  readonly url: string;
  /** Files in `texts/`. */
  readonly texts: readonly string[];
  /** Standard texts in `spdx/` to add, for licenses the library's own files only name. */
  readonly spdx?: readonly string[];
  readonly version: NativeVersion;
  /**
   * The archives of `packages/desktop/sources.json` that hold the source of what is shipped,
   * which a copyleft license obliges Aspen to offer: the library's own, and the one holding the
   * scripts and patches it was built with (obs-deps', or Electron's).
   */
  readonly sources?: readonly string[];
}

/**
 * Where a library's version comes from, and so what the build checks it against: OBS Studio's
 * Windows release of this version, which the build checks is still the one fetched, shipping
 * OBS Studio itself or a library it bundles (`bundled`), whose version is whatever that release
 * carries; Electron of this version, which the build checks is the one installed, whose release
 * carries the library; or the Flatpak's base app at this commit, which the build checks is the one
 * pinned, carrying the library at its own version (`own`).
 */
export type NativeVersion =
  | { readonly obs: string; readonly bundled: boolean }
  | { readonly electron: string }
  | { readonly flatpak: string; readonly own: string };

const OBS_VERSION = "32.2.2";
const ELECTRON_VERSION = "44.9.1";
const BASE_APP_COMMIT = "cd83a25d32aef701b5cb2f546350598fdd08540f0f884717f6f4baa476d292f1";
/** The base app's own repository at the commit it was built from: its manifest and patches. */
const BASE_APP_SOURCE =
  "org.electronjs.Electron2.BaseApp-11eef849032145d068097de17c67211154f6526c.tar.gz";
/** Flathub's modules the base app builds some libraries from, with their patches. */
const SHARED_MODULES = "flathub-shared-modules-41d0ece01788b2655aa93c1d7440cd63a3d358ba.tar.gz";

/**
 * OBS Studio's libobs and the libraries of its release the Windows desktop app ships beside it
 * (`fetch-libobs.mjs`, which says which).
 */
const OBS: readonly NativeLibrary[] = [
  {
    component: "desktop",
    name: "OBS Studio (libobs and its modules)",
    license: "GPL-2.0-or-later",
    url: "https://obsproject.com",
    texts: ["obs-studio-COPYING"],
    version: { obs: OBS_VERSION, bundled: false },
    sources: ["OBS-Studio-32.2.2-Sources.tar.gz", "obs-deps-2026-07-15.tar.gz"],
  },
  {
    component: "desktop",
    // Built into `obs.dll`, from OBS Studio's own source (`deps/libcaption`).
    name: "libcaption",
    license: "MIT",
    url: "https://github.com/szatmary/libcaption",
    texts: ["libcaption-LICENSE.txt"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    // Built into the Quick Sync encoder module and its test program (`obs-qsv11`, `obs-qsv-test.exe`).
    name: "Intel Video Processing Library (libvpl)",
    license: "MIT",
    url: "https://github.com/intel/libvpl",
    texts: ["libvpl-LICENSE"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    // Built into the game capture hook (`graphics-hook64.dll`).
    name: "Microsoft Detours",
    license: "MIT",
    url: "https://github.com/microsoft/Detours",
    texts: ["detours-LICENSE.md"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    name: "FFmpeg",
    // Built with `--enable-gpl --enable-version3` (obs-deps' `99-ffmpeg.ps1`).
    license: "GPL-3.0-or-later",
    url: "https://ffmpeg.org",
    texts: ["ffmpeg-LICENSE.md", "ffmpeg-COPYING.GPLv3"],
    version: { obs: OBS_VERSION, bundled: true },
    sources: [
      "ffmpeg-38b88335f99e76ed89ff3c93f877fdefce736c13.tar.gz",
      "obs-deps-2026-07-15.tar.gz",
    ],
  },
  {
    component: "desktop",
    // Built into FFmpeg's libraries (`--enable-libmp3lame`).
    name: "LAME",
    license: "LGPL-2.0-or-later",
    url: "https://lame.sourceforge.io",
    texts: ["lame-COPYING"],
    version: { obs: OBS_VERSION, bundled: true },
    sources: ["lame-3.100.tar.gz", "obs-deps-2026-07-15.tar.gz"],
  },
  {
    component: "desktop",
    // Built into FFmpeg's libraries (`--enable-libvpx`), as is each library after it up to x264.
    name: "libvpx",
    license: "BSD-3-Clause",
    url: "https://www.webmproject.org/code/",
    texts: ["libvpx-LICENSE", "libvpx-PATENTS"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    // `--enable-libaom`.
    name: "AOM AV1 codec library (libaom)",
    license: "BSD-2-Clause",
    url: "https://aomedia.googlesource.com/aom/",
    texts: ["aom-LICENSE", "aom-PATENTS"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    // `--enable-libsvtav1`.
    name: "SVT-AV1",
    license: "BSD-3-Clause-Clear",
    url: "https://gitlab.com/AOMediaCodec/SVT-AV1",
    texts: ["svt-av1-LICENSE.md", "svt-av1-PATENTS.md"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    // `--enable-libopus`.
    name: "Opus",
    license: "BSD-3-Clause",
    url: "https://opus-codec.org",
    texts: ["opus-COPYING"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    // `--enable-libvorbis`.
    name: "libvorbis",
    license: "BSD-3-Clause",
    url: "https://xiph.org/vorbis/",
    texts: ["libvorbis-COPYING"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    // Built with libvorbis and libtheora.
    name: "libogg",
    license: "BSD-3-Clause",
    url: "https://xiph.org/ogg/",
    texts: ["libogg-COPYING"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    // `--enable-libtheora`.
    name: "libtheora",
    license: "BSD-3-Clause",
    url: "https://www.theora.org",
    texts: ["libtheora-COPYING", "libtheora-LICENSE"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    // FFmpeg's NVENC encoders, and the NVENC module and its test program, are built from them; each header carries its own notice.
    name: "NVIDIA codec headers (nv-codec-headers)",
    license: "MIT",
    url: "https://github.com/FFmpeg/nv-codec-headers",
    texts: ["nv-codec-headers-dynlink_loader.h", "nv-codec-headers-nvEncodeAPI.h"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    // FFmpeg's AMF encoders, and their test program, are built from its headers.
    name: "AMD Advanced Media Framework (AMF)",
    license: "MIT",
    url: "https://github.com/GPUOpen-LibrariesAndSDKs/AMF",
    texts: ["amf-LICENSE.txt"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    name: "x264",
    license: "GPL-2.0-or-later",
    url: "https://www.videolan.org/developers/x264.html",
    texts: ["x264-COPYING"],
    version: { obs: OBS_VERSION, bundled: true },
    sources: ["x264-eaa68fad9e5d201d42fde51665f2d137ae96baf0.tar.gz", "obs-deps-2026-07-15.tar.gz"],
  },
  {
    component: "desktop",
    name: "libcurl",
    license: "curl",
    url: "https://curl.se",
    texts: ["curl-COPYING"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    name: "libdatachannel",
    license: "MPL-2.0",
    url: "https://github.com/paullouisageneau/libdatachannel",
    texts: ["libdatachannel-LICENSE"],
    version: { obs: OBS_VERSION, bundled: true },
    sources: [
      "libdatachannel-4e4f4892dccb2a57fe3a490d0c9d958de4244e74.tar.gz",
      "obs-deps-2026-07-15.tar.gz",
    ],
  },
  {
    component: "desktop",
    // Built into libdatachannel's library, as the submodule its source names.
    name: "libjuice",
    license: "MPL-2.0",
    url: "https://github.com/paullouisageneau/libjuice",
    texts: ["libjuice-LICENSE"],
    version: { obs: OBS_VERSION, bundled: true },
    sources: ["libjuice-5948a4162d37bc213d6051b67ee2876ccc5a99a6.tar.gz"],
  },
  {
    component: "desktop",
    // Built into libdatachannel's library, as are usrsctp and plog.
    name: "libsrtp",
    license: "BSD-3-Clause",
    url: "https://github.com/cisco/libsrtp",
    texts: ["libsrtp-LICENSE"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    // As libsrtp.
    name: "usrsctp",
    license: "BSD-3-Clause",
    url: "https://github.com/sctplab/usrsctp",
    texts: ["usrsctp-LICENSE.md"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    // As libsrtp.
    name: "plog",
    license: "MIT",
    url: "https://github.com/SergiusTheBest/plog",
    texts: ["plog-LICENSE"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    name: "librist",
    license: "BSD-2-Clause",
    url: "https://code.videolan.org/rist/librist",
    texts: ["librist-COPYING"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    name: "SRT",
    license: "MPL-2.0",
    url: "https://github.com/Haivision/srt",
    texts: ["srt-LICENSE"],
    version: { obs: OBS_VERSION, bundled: true },
    sources: ["srt-1.5.2.zip", "obs-deps-2026-07-15.tar.gz"],
  },
  {
    component: "desktop",
    // Built into SRT's, librist's, and libdatachannel's libraries.
    name: "Mbed TLS",
    license: "Apache-2.0 OR GPL-2.0-or-later",
    url: "https://www.trustedfirmware.org/projects/mbed-tls/",
    texts: ["mbedtls-LICENSE"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    name: "pthreads-win32",
    license: "LGPL-2.1-or-later",
    url: "https://sourceware.org/pthreads-win32/",
    texts: ["pthreads-win32-COPYING", "pthreads-win32-COPYING.LIB"],
    version: { obs: OBS_VERSION, bundled: true },
    // Kept in OBS Studio's own source (`deps/w32-pthreads`).
    sources: ["OBS-Studio-32.2.2-Sources.tar.gz"],
  },
  {
    component: "desktop",
    name: "zlib",
    license: "Zlib",
    url: "https://zlib.net",
    texts: ["zlib-LICENSE"],
    version: { obs: OBS_VERSION, bundled: true },
  },
];

/**
 * What Electron ships beside Chromium as a library of its own, and so under that library's
 * license rather than among Chromium's notices: FFmpeg, as Chromium's copy of it, built as
 * `ffmpeg.dll` (`libffmpeg.so`, `libffmpeg.dylib`) by Electron's release configuration.
 */
const ELECTRON: readonly NativeLibrary[] = [
  {
    component: "desktop",
    name: "FFmpeg (Chromium's, in Electron)",
    license: "LGPL-2.1-or-later",
    url: "https://chromium.googlesource.com/chromium/third_party/ffmpeg",
    texts: ["chromium-ffmpeg-LICENSE.md", "chromium-ffmpeg-COPYING.LGPLv2.1"],
    version: { electron: ELECTRON_VERSION },
    // The revision Chromium's `DEPS` pins at the Chromium version Electron's `DEPS` names, and
    // Electron's source, which holds its patch to it and the build arguments it is built with.
    sources: [
      "chromium-ffmpeg-2b68d2babae73714846961fb0ee47e3b3d2e39a9.tar.gz",
      "electron-44.9.1.tar.gz",
    ],
  },
];

/**
 * What the Flatpak carries from its base app, `org.electronjs.Electron2.BaseApp`, which Chromium
 * finds and loads when they are there: zypak, which the app runs through, the keyring libraries,
 * the tray's, the launcher's, and speech's. The base app's build tools are removed from the
 * package (`cleanup-commands` in the manifest), so they are not listed.
 */
const FLATPAK: readonly NativeLibrary[] = [
  {
    component: "desktop",
    // Runs Chromium's sandbox inside Flatpak's; the app starts through it (`flatpak/aspen.sh`).
    name: "zypak",
    license: "BSD-3-Clause",
    url: "https://github.com/refi64/zypak",
    texts: ["zypak-LICENSE"],
    version: { flatpak: BASE_APP_COMMIT, own: "693a71c5ffa8" },
    sources: [BASE_APP_SOURCE],
  },
  {
    component: "desktop",
    name: "libsecret",
    license: "LGPL-2.1-or-later",
    url: "https://gitlab.gnome.org/GNOME/libsecret",
    texts: ["libsecret-COPYING"],
    version: { flatpak: BASE_APP_COMMIT, own: "0.21.7" },
    sources: ["libsecret-0.21.7.tar.xz", BASE_APP_SOURCE],
  },
  {
    component: "desktop",
    name: "libgnome-keyring",
    license: "LGPL-2.0-or-later",
    url: "https://gitlab.gnome.org/Archive/libgnome-keyring",
    texts: ["libgnome-keyring-COPYING"],
    version: { flatpak: BASE_APP_COMMIT, own: "3.12.0" },
    sources: ["libgnome-keyring-3.12.0.tar.xz", BASE_APP_SOURCE],
  },
  {
    component: "desktop",
    name: "libappindicator",
    license: "LGPL-2.1-only OR LGPL-3.0-only",
    url: "https://launchpad.net/libappindicator",
    texts: ["libappindicator-COPYING", "libappindicator-COPYING.LGPL.2.1"],
    version: { flatpak: BASE_APP_COMMIT, own: "12.10.0" },
    sources: ["libappindicator-12.10.0.tar.gz", BASE_APP_SOURCE, SHARED_MODULES],
  },
  {
    component: "desktop",
    name: "libindicator",
    license: "GPL-3.0-only",
    url: "https://launchpad.net/libindicator",
    texts: ["libindicator-COPYING"],
    version: { flatpak: BASE_APP_COMMIT, own: "12.10.1" },
    sources: ["libindicator-12.10.1.tar.gz", BASE_APP_SOURCE, SHARED_MODULES],
  },
  {
    component: "desktop",
    name: "libdbusmenu",
    license: "LGPL-2.1-only OR LGPL-3.0-only",
    url: "https://launchpad.net/libdbusmenu",
    texts: ["libdbusmenu-COPYING", "libdbusmenu-COPYING.2.1"],
    version: { flatpak: BASE_APP_COMMIT, own: "16.04.0" },
    sources: ["libdbusmenu-16.04.0.tar.gz", BASE_APP_SOURCE, SHARED_MODULES],
  },
  {
    component: "desktop",
    name: "dbus-glib",
    license: "AFL-2.1 OR GPL-2.0-or-later",
    url: "https://gitlab.freedesktop.org/dbus/dbus-glib",
    texts: [
      "dbus-glib-COPYING",
      "dbus-glib-LICENSES-AFL-2.1.txt",
      "dbus-glib-LICENSES-GPL-2.0-or-later.txt",
      "dbus-glib-LICENSES-LGPL-2.1-or-later.txt",
      "dbus-glib-LICENSES-MIT.txt",
    ],
    version: { flatpak: BASE_APP_COMMIT, own: "0.114" },
    sources: ["dbus-glib-0.114.tar.gz", BASE_APP_SOURCE, SHARED_MODULES],
  },
  {
    component: "desktop",
    // The library is LGPL-3.0; its extras library (`libunity-extras`) GPL-3.0.
    name: "libunity",
    license: "LGPL-3.0-only AND GPL-3.0-only",
    url: "https://launchpad.net/libunity",
    texts: ["libunity-COPYING", "libunity-COPYING.GPL-3"],
    version: { flatpak: BASE_APP_COMMIT, own: "7.1.4" },
    sources: ["libunity_7.1.4+19.04.20190319.orig.tar.gz", BASE_APP_SOURCE],
  },
  {
    component: "desktop",
    name: "Dee",
    license: "LGPL-3.0-only",
    url: "https://launchpad.net/dee",
    texts: ["dee-COPYING"],
    version: { flatpak: BASE_APP_COMMIT, own: "1.2.7" },
    sources: ["dee-1.2.7.tar.gz", BASE_APP_SOURCE],
  },
  {
    component: "desktop",
    name: "libspeechd (Speech Dispatcher's client library)",
    license: "LGPL-2.1-or-later",
    url: "https://github.com/brailcom/speechd",
    texts: ["speech-dispatcher-COPYING.LGPL"],
    version: { flatpak: BASE_APP_COMMIT, own: "0.12.1" },
    sources: ["speech-dispatcher-0.12.1.tar.gz", BASE_APP_SOURCE],
  },
];

export const NATIVE_LIBRARIES: readonly NativeLibrary[] = [...OBS, ...ELECTRON, ...FLATPAK];

/**
 * Texts for packages whose published files carry none, by package name, taken from their
 * projects' sources; preferred to a standard text, since they name the copyright holders.
 */
export const PACKAGE_TEXTS: Readonly<Record<string, readonly string[]>> = {};
