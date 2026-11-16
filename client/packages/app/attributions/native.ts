/**
 * What `plugin.ts` cannot find in a package manager's records: the C and C++ libraries built or
 * shipped beside the packages it walks, and license texts for packages that carry none. Each
 * text named here is a file in `texts/` (taken from the library's own source at the version
 * shipped) or a standard text in `spdx/`.
 *
 * Every entry is tied to where its version is decided, and the build stops when that moves, so
 * an update cannot ship with this list describing the version before it: what the Windows
 * desktop app ships from OBS Studio's release, to `OBS_VERSION` in
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

/**
 * Where a library's version comes from, and so what the build checks it against: OBS Studio's
 * Windows release of this version, which the build checks is still the one fetched, shipping
 * OBS Studio itself or a library it bundles (`bundled`), whose version is whatever that release
 * carries.
 */
export interface NativeVersion {
  readonly obs: string;
  readonly bundled: boolean;
}

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

export const NATIVE_LIBRARIES: readonly NativeLibrary[] = OBS;

/**
 * Texts for packages whose published files carry none, by package name, taken from their
 * projects' sources; preferred to a standard text, since they name the copyright holders.
 */
export const PACKAGE_TEXTS: Readonly<Record<string, readonly string[]>> = {};
