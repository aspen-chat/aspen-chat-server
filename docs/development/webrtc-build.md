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
scripts/webrtc_build.py status
```

A first `checkout` fetches the source and its dependency tree without history (about 25 minutes and
30 GB on a home connection); a later one, after the pin moves, fetches the new branch alone. Keep
the checkout between builds.

`--jobs` caps the compile jobs at once. Google's build runs one per core, and each heavy C++ file
takes up to 1.5 GB while it compiles, so a machine with 8 GB builds at 3: on an 8 GB M1 Mac both iOS
slices build in about 17 minutes that way, with free memory at the floor but swap left as it was. A
16 GB machine needs no cap.

## Moving the pin

A milestone is a `branch-heads/N` branch of Google's repository, N being the Chromium branch of that
milestone: `webrtc_branch` in `https://chromiumdash.appspot.com/fetch_milestones?mstone=M`. Change
the two constants, read Google's release notes for every milestone crossed, run `checkout` and
`build` on both platforms, and run the call engine's device checks before the apps take the new
artifacts.

## What the script does to Google's tools

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
