# Building WebRTC for the mobile apps

The mobile apps' call engine (`voice_client`) links Google's WebRTC library, built from Google's
source at one pinned milestone with `scripts/webrtc_build.py`, which drives Google's own build
tools. The apps link no prebuilt package: what parses media from the network on a phone is source we
chose, moved to a newer milestone on our own schedule.

| What                 | Where                                                                      |
| -------------------- | -------------------------------------------------------------------------- |
| The pinned milestone | `WEBRTC_BRANCH` and `WEBRTC_MILESTONE` in `scripts/webrtc_build.py`        |
| iOS                  | Built on a Mac: `WebRTC.xcframework`, device arm64 and simulator arm64     |
| Android              | Built on Linux: `libwebrtc.aar`, `arm64-v8a`, `armeabi-v7a`, and `x86_64`  |
| The checkout         | `~/webrtc-build` (`--root`), 30 GB and more, beside Google's `depot_tools` |
| The artifact         | `target/webrtc/` (`--out`)                                                 |

```
scripts/webrtc_build.py checkout          fetch, or move the checkout to the pinned branch
scripts/webrtc_build.py build --jobs 3    build this machine's platform
scripts/webrtc_build.py package           pack the Android archive again from the last build
scripts/webrtc_build.py status
```

A first `checkout` fetches the source and its dependency tree without history (about 25 minutes and
30 GB on a home connection); a later one, after the pin moves, fetches the new branch alone. Keep
the checkout between builds.

`--jobs` caps the compile jobs at once. Google's build runs one per core, and each heavy C++ file
takes up to 1.5 GB while it compiles, so a machine with 8 GB builds at 3: on an 8 GB M1 Mac both iOS
slices build in about 17 minutes that way, with free memory at the floor but swap left as it was. A
16 GB machine needs no cap.

## In continuous integration

`.github/workflows/ci.yml` builds both artifacts in a job per platform, `webrtc-android` on
`ubuntu-24.04` and `webrtc-ios` on `macos-26`, which the Android and iOS jobs wait for and take them
from as the artifacts `webrtc-android` (`libwebrtc.aar`) and `webrtc-ios` (`WebRTC.xcframework` as
`webrtc-ios.tar.gz`, since an artifact keeps no file modes). Each is cached with `actions/cache`
under `target/webrtc/`, keyed `webrtc-<platform>-<WEBRTC_BRANCH>-<hash of this script>`, so a run
builds only when the pin or the script has changed since the last build on `main` or that branch, or
when GitHub has evicted the cache (one unused for seven days), and a cache hit takes seconds. The
checkout is never cached, and never fits the 10 GB a repository's caches share:

| Platform | Runner                                                    | The checkout                                                                                                                                 | The build                                                                         |
| -------- | --------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| Android  | 4 cores, 16 GB, root disk ~20 GB free, `/mnt` ~66 GB free | 40 GB without history, 1.4 GB of `depot_tools`, 2 GB of output: on `/mnt` (`--root /mnt/webrtc-build`), after a check that it has 50 GB free | 4.7 minutes on 32 cores here; the runner's four are allowed 150 for the whole job |
| iOS      | 3 cores (M1), 7 GB, ~170 GB free                          | About 30 GB, in `~/webrtc-build`                                                                                                             | `--jobs 3`, as on an 8 GB M1 (about 17 minutes); 150 for the job                  |

A pull request that moves the pin builds on its own runners and its cache stays with its branch; the
first run on `main` after it merges builds once more and every run after takes the cache.

## Moving the pin

A milestone is a `branch-heads/N` branch of Google's repository, N being the Chromium branch of that
milestone: `webrtc_branch` in `https://chromiumdash.appspot.com/fetch_milestones?mstone=M`. Change
the two constants, read Google's release notes for every milestone crossed, run `checkout` and
`build` on both platforms, and run the call engine's device checks before the apps take the new
artifacts.

## What the script does to Google's tools

- On Android, it packs the archive again after Google's `build_aar.py` (`package` does this step
  alone), adding the Java classes that archive leaves out. The Java API's native methods are
  declared on classes jni_zero generates: `org.webrtc.*Jni`, one per class with natives, and `J.N`,
  the one class whose natives the library exports under hashed names. Google's `dist_jar` takes its
  direct dependencies alone, so the `*Jni` classes are left out of `classes.jar`, and `J.N` is
  generated for each application that links the library (`jni_zero.py generate-final`, which the
  build runs for Google's own example apps and no one else), so an app linking the archive as it
  comes fails on its first native method with `NoClassDefFoundError`. The script takes the `*Jni`
  classes from the jars the build compiled them into, runs the generator for `J.N` with the command
  the build wrote for the library's own registration target (whose flags must be the build's, since
  the library exports what they say), compiles it against the archive's classes with the checkout's
  JDK, and adds both to `classes.jar`. Its scratch files are `target/webrtc/android-jni/`.

- It puts `depot_tools`' own Python first on the path. macOS's `/usr/bin/python3` is a shim that
  resolves through Xcode's developer directory, and the Chromium toolchain's linker driver runs
  `python3` with that directory pointed at a linked SDK, where the shim fails to find anything.
- With `--jobs`, the iOS build script is edited for the one build and restored after: it runs
  Google's build driver, `siso`, with no way to say how many jobs, and the cap is `-local_jobs`. The
  Android script takes the switch itself.
- Every build starts from nothing. Build state kept between builds names the Xcode SDK it was made
  with, and an Xcode update between builds removes that SDK.
- On iOS, linker warnings are not fatal. The build's host tools are linked with clang's default
  library path `/usr/local/lib` among the search paths, which the macOS SDK does not have; lld warns
  of the missing directory, which nothing needs.
