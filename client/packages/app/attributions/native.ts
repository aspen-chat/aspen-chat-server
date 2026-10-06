/**
 * What `plugin.ts` cannot find in a package manager's records: the C and C++ libraries built or
 * shipped beside the packages it walks, and license texts for packages that carry none. Each
 * text named here is a file in `texts/` (taken from the library's own source at the version
 * shipped) or a standard text in `spdx/`.
 *
 * Every entry is tied to where its version is decided, and the build stops when that moves, so
 * an update cannot ship with this list describing the version before it: mediasoup's
 * subprojects to the Meson wraps in the `mediasoup-sys` crate the server locks, and what the
 * Windows desktop app ships from OBS Studio's release to `OBS_VERSION` in
 * `packages/desktop/scripts/fetch-libobs.mjs`.
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
}

/** Where a library's version comes from, and so what the build checks it against. */
export type NativeVersion =
  /** The `directory` of `subprojects/<wrap>.wrap` in the locked `mediasoup-sys`, as recorded. */
  | { readonly wrap: string; readonly directory: string; readonly version: string }
  /**
   * Shipped from OBS Studio's Windows release of this version, which the build checks is still
   * the one fetched: OBS Studio itself, or a library it bundles (`bundled`), whose version is
   * whatever that release carries.
   */
  | { readonly obs: string; readonly bundled: boolean };

/** The libraries mediasoup's worker, in the voice server, builds from Meson wraps. */
const MEDIASOUP: readonly NativeLibrary[] = [
  {
    component: "server",
    name: "abseil-cpp",
    license: "Apache-2.0",
    url: "https://abseil.io",
    texts: ["abseil-cpp-LICENSE"],
    version: { wrap: "abseil-cpp", directory: "abseil-cpp-20240722.0", version: "20240722.0" },
  },
  {
    component: "server",
    name: "FlatBuffers",
    license: "Apache-2.0",
    url: "https://flatbuffers.dev",
    texts: ["flatbuffers-LICENSE"],
    version: { wrap: "flatbuffers", directory: "flatbuffers-24.3.25", version: "24.3.25" },
  },
  {
    component: "server",
    name: "libsrtp",
    license: "BSD-3-Clause",
    url: "https://github.com/versatica/libsrtp",
    texts: ["libsrtp-LICENSE"],
    version: {
      wrap: "libsrtp3",
      directory: "libsrtp-3.0.0-beta-2fc078db",
      version: "3.0.0-beta-2fc078db",
    },
  },
  {
    component: "server",
    name: "libuv",
    license: "MIT",
    url: "https://libuv.org",
    texts: ["libuv-LICENSE", "libuv-LICENSE-extra"],
    version: { wrap: "libuv", directory: "libuv-v1.51.0", version: "1.51.0" },
  },
  {
    component: "server",
    name: "OpenSSL",
    license: "Apache-2.0",
    url: "https://www.openssl.org",
    texts: ["openssl-LICENSE.txt"],
    version: { wrap: "openssl", directory: "openssl-3.0.8", version: "3.0.8" },
  },
  {
    component: "server",
    name: "unordered_dense",
    license: "MIT",
    url: "https://github.com/martinus/unordered_dense",
    texts: ["unordered_dense-LICENSE"],
    version: { wrap: "unordered-dense", directory: "unordered_dense-4.8.1", version: "4.8.1" },
  },
];

/**
 * The wraps mediasoup has that no shipped build compiles: its tests' framework, and Windows'
 * `getopt`, the voice server running on Linux alone.
 */
export const UNSHIPPED_WRAPS: readonly string[] = ["catch2", "wingetopt"];

const OBS_VERSION = "32.2.2";

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
  },
  {
    component: "desktop",
    name: "FFmpeg",
    license: "LGPL-2.1-or-later",
    url: "https://ffmpeg.org",
    texts: ["ffmpeg-LICENSE.md"],
    spdx: ["LGPL-2.1-or-later"],
    version: { obs: OBS_VERSION, bundled: true },
  },
  {
    component: "desktop",
    name: "x264",
    license: "GPL-2.0-or-later",
    url: "https://www.videolan.org/developers/x264.html",
    texts: ["x264-COPYING"],
    version: { obs: OBS_VERSION, bundled: true },
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
  },
  {
    component: "desktop",
    name: "pthreads-win32",
    license: "LGPL-2.1-or-later",
    url: "https://sourceware.org/pthreads-win32/",
    texts: ["pthreads-win32-COPYING", "pthreads-win32-COPYING.LIB"],
    version: { obs: OBS_VERSION, bundled: true },
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

export const NATIVE_LIBRARIES: readonly NativeLibrary[] = [...MEDIASOUP, ...OBS];

/**
 * Texts for packages whose published files carry none, by package name, taken from their
 * projects' sources; preferred to a standard text, since they name the copyright holders.
 */
export const PACKAGE_TEXTS: Readonly<Record<string, readonly string[]>> = {
  mediasoup: ["mediasoup-LICENSE"],
  "mediasoup-sys": ["mediasoup-LICENSE"],
  "mediasoup-types": ["mediasoup-LICENSE"],
};
