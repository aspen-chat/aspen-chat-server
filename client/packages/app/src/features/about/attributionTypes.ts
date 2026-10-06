/**
 * The shape of `virtual:attributions`, which `attributions/plugin.ts` writes when the app is
 * built: every package Aspen's apps and servers are made from, by the part of Aspen that ships
 * it, with the license texts they carry, each text kept once however many packages share it.
 */
export interface Attributions {
  readonly components: readonly AttributedComponent[];
  /** The texts packages refer to by index. */
  readonly texts: readonly string[];
  /**
   * What a development server could not list (the Rust crates without `cargo`, say), so the
   * page says the list is short. A build stops instead, so this is empty in every build.
   */
  readonly incomplete: readonly string[];
}

/**
 * The part of Aspen a package ships in: the app every shell renders, the desktop app's own
 * binaries, the phone apps' native code, and the servers.
 */
export type ComponentId = "app" | "desktop" | "mobile" | "server";

export interface AttributedComponent {
  readonly id: ComponentId;
  readonly packages: readonly AttributedPackage[];
}

export interface AttributedPackage {
  readonly name: string;
  /**
   * `null` where it is whatever another release carries: Chromium's is Electron's, and the
   * libraries the desktop app ships from OBS Studio are its release's (`bundledIn`).
   */
  readonly version: string | null;
  /** The release it ships as part of, whose version decides its own. */
  readonly bundledIn?: string;
  /** As the package declares it, an SPDX expression where it gives one. */
  readonly license: string;
  /** Its home page or repository. */
  readonly url: string | null;
  readonly texts: readonly LicenseFile[];
  /**
   * Whether the texts are the standard wording of its license, the package having published
   * none of its own.
   */
  readonly standardText: boolean;
  /** Chromium's notices, which ship beside the desktop app (`LICENSES.chromium.html`). */
  readonly chromium?: true;
}

export interface LicenseFile {
  /** The file's path within the package, or the license's identifier for a standard text. */
  readonly file: string;
  /** Its index in `Attributions.texts`. */
  readonly text: number;
}

/** What `virtual:build-info` holds: this build of the app. */
export interface BuildInfo {
  readonly version: string;
  /** The commit it was built from, abbreviated; `null` outside a git checkout. */
  readonly commit: string | null;
  /** Whether the checkout had changes beyond that commit. */
  readonly modified: boolean;
  /**
   * Where its source code is published (`repository` in the client's `package.json`), at the
   * commit it was built from when that is known and unchanged.
   */
  readonly source: string;
}
