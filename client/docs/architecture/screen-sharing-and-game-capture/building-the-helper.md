# Building the helper

The helper is built by `pnpm build:native` in `packages/desktop`. On Linux and macOS that can run
while a shell is open.

## What each platform needs

| Platform | Links libobs | Build needs |
| --- | --- | --- |
| Windows | Always (`src/obs/`, behind `cfg(obs)`) | The bundled libobs from `pnpm fetch:libobs` (below). |
| Linux | Only with the `libobs` feature | PipeWire's and libopus's development files. No libobs. |
| macOS | Only with the `libobs` feature | Xcode's toolchain and `cmake`. libopus is built from source. |

The `libobs` feature on Linux and macOS is for developing the picture path there.

## Windows

On Windows the libobs is the build's own. `pnpm fetch:libobs` (`scripts/fetch-libobs.mjs`) lays the
OBS Project's release, pinned by version and SHA-256, out in `native/libobs` (gitignored).

**Its files are exactly as the OBS Project signed them.** Anti-cheat systems whitelist the injected
game hook by that signature.

`native/libobs` holds, where the helper's build script finds them:

- `obs.dll` and the libraries it and the modules need;
- the five modules the helper loads;
- their data, with the hook's files;
- headers from the same tag's source;
- `obs.lib`, written from `obs.dll`'s exports (MSVC's `lib` or LLVM's `llvm-dlltool`).

Then:

- The installer ships that directory as the `libobs` resource (`electron-builder.yml`).
- The shell starts the helper with its libraries on the `PATH`, and names its modules and data in
  every request (`bundledLibobs` in `gameCapture.ts`).
- CI packages the Windows installer that way.

## The `libobs` feature on Linux and macOS

With the `libobs` feature, the helper needs libobs's development files.

### Linux

`pkg-config` finds them.

### macOS

`LIBOBS_INCLUDE_DIR` and `LIBOBS_LIB_DIR` name them:

- `LIBOBS_INCLUDE_DIR`: the `libobs` directory of an OBS Studio source checkout at the installed
  version, with an `obsconfig.h` written from its `.in`. Use OBS.app's `Contents/PlugIns` and
  `Contents/Resources/data` as the paths.
- `LIBOBS_LIB_DIR`: a directory holding a `libobs.dylib` link to OBS.app's
  `Contents/Frameworks/libobs.framework/Versions/A/libobs`, of the same architecture as the helper.

Also:

- The helper carries that Frameworks directory as its rpath.
- Set `BINDGEN_EXTRA_CLANG_ARGS=-I/opt/homebrew/include` for `simde` (`brew install simde`), which
  the headers include on ARM.
- The helper then expects OBS at `/Applications/OBS.app` at run time.
