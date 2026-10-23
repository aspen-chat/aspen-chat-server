# Android builds in CI

CI's `android` job in `.github/workflows/ci.yml` builds the Android app, runs its JVM tests and its
device tests on an emulator, and keeps the APK it built as the artifact `android-apk-debug` on every
pull request and every push to `main`, for 14 days.

## The APK

It is the debug build, signed with the runner's own debug key, with no push: it names no relay and
carries no `google-services.json` (`push.md`). It is one APK for every ABI, since the app has no
native code of its own. The artifact is uploaded before the device tests run, so a failing emulator
still leaves it to install.

Install it on a device with USB debugging on:

```
unzip android-apk-debug.zip && adb install -r app-debug.apk
```

Each runner makes a debug key of its own, so an APK from one run installs over another's only after
the one installed is removed (`adb uninstall com.aspen_chat.client`).

## Versions

`app/build.gradle` reads two environment variables, which the job sets:

| Variable             | Becomes                                                      | Unset |
| -------------------- | ------------------------------------------------------------ | ----- |
| `ASPEN_BUILD_NUMBER` | `versionCode` (the workflow's run number)                    | `1`   |
| `ASPEN_BUILD_COMMIT` | build metadata on `versionName`, its first twelve characters | none  |

`versionName` is `packages/mobile/package.json`'s version, so a CI build reads `0.1.0+ce1a1be2abcd`
in About Aspen (`about.md`), and names the commit it was built from. A later run's `versionCode` is
higher, so a build signed with the same key as the one installed updates it in place.
