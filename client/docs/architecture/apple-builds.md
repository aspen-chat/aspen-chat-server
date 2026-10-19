# Apple builds in CI

CI builds the iOS app and the macOS desktop app on GitHub's macOS runners (`macos-26`, Apple Silicon). A pull request's builds are unsigned. A push to `main` also signs both: the desktop app's dmg with the Developer ID Application identity, notarized, and the iOS app for the App Store, which can be sent to TestFlight.

## Where it lives

| Part                                          | Code                                                                                               |
| --------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| The jobs                                      | `desktop-macos`, `desktop-macos-signed`, `ios`, and `ios-testflight` in `.github/workflows/ci.yml` |
| Signing the dmg                               | `mac` in `packages/desktop/electron-builder.signed.yml`                                            |
| Archiving, signing, and uploading the iOS app | `packages/mobile/ios/scripts/app_store.py`                                                         |

## The jobs

| Job                    | Runs on   | What it does                                                                                                                                                                                                                           |
| ---------------------- | --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `desktop-macos`        | every run | The capture helper's clippy and tests on macOS. On a pull request, also unsigned dmgs, kept as an artifact                                                                                                                             |
| `desktop-macos-signed` | `main`    | Dmgs signed and notarized, then checked for each app: its architecture, a universal capture helper, the identity, the hardened runtime and a timestamp on the app and the capture helper, the stapled ticket, and Gatekeeper's verdict |
| `ios`                  | every run | The app and its UI tests built for the simulator, unsigned, and `check_webpush.sh`                                                                                                                                                     |
| `ios-testflight`       | `main`    | An App Store build, sent to App Store Connect while `TESTFLIGHT_UPLOAD` is `true`; otherwise its `.ipa` is kept as an artifact                                                                                                         |

The UI tests themselves are not run: they need a whole deployment (`ios-app.md`), and these runners have no Docker to run one in.

There are two dmgs, `Aspen-<version>-arm64.dmg` for Apple Silicon and `Aspen-<version>-x64.dmg` for Intel, both built on the Apple Silicon runner. **Why not one universal app:** every download would carry both architectures' Electron, about twice the size. The capture helper is small, so it is one universal binary that both apps carry (`scripts/build-native.mjs`).

## Secrets and variables

The signing jobs take their secrets from the GitHub Environment `apple-signing`, which only `main` may use. **Why:** a pull request's code is not reviewed when it runs, and whatever is signed carries the publisher's name.

| Secret                                         | What it holds                                                                                                                                                         |
| ---------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `APPLE_API_KEY`                                | An App Store Connect API key's `.p8` file, with the Developer role. It notarizes the dmg and uploads to TestFlight                                                    |
| `APPLE_API_KEY_ID`, `APPLE_API_ISSUER`         | That key's ID and its issuer's                                                                                                                                        |
| `MAC_CERT_P12`, `MAC_CERT_PASSWORD`            | The Developer ID Application certificate and its key as a base64 `.p12`, and the file's password                                                                      |
| `IOS_CERT_P12`, `IOS_CERT_PASSWORD`            | The Apple Distribution certificate and its key in the same form                                                                                                       |
| `IOS_PROFILE_APP`, `IOS_PROFILE_NOTIFICATIONS` | The App Store provisioning profiles for `com.aspen-chat.client` and `com.aspen-chat.client.notifications`, base64, each made with that Apple Distribution certificate |

Two repository variables, which are not secret:

| Variable            | Effect                                                                                                    |
| ------------------- | --------------------------------------------------------------------------------------------------------- |
| `TESTFLIGHT_UPLOAD` | `true` sends each `main` build to App Store Connect. Anything else exports the signed `.ipa` and keeps it |
| `ASPEN_PUSH_RELAY`  | The relay the iOS build names (`spec/push.md`). Unset, the build has no push                              |

When a certificate is renewed, its profiles must be made again with the new one, and all three secrets replaced. `app_store.py` names the profile that has expired or lacks the identity's certificate.

## The iOS App Store build

`app_store.py` signs by hand. **Why not Xcode's automatic signing:** with an API key in CI, automatic signing makes a new development certificate on every run, and an account may hold only a few.

What it does:

1. It reads each profile, checks that it is an App Store profile of team `L2GS25U79G`, for one of the two bundles, and not expired, and installs it.
2. It imports the `.p12` into a keychain of its own, deleted at the end. Without `--certificate` it uses the identities already in the keychains, for archiving from a Mac where they are.
3. It archives the `App` scheme. A generated xcconfig gives each target the profile of its own bundle id (`ASPEN_PROFILE_<bundle id as an identifier>`). A setting on the command line would give every target the same profile, Swift packages included, which take none.
4. It exports for `app-store-connect`, uploading with the API key when given `--upload`.

The build number is `<run number>.<attempt>`, so each upload of a version is greater than the last. The extension's `Info.plist` carries the app's version and build number: App Store Connect refuses an app whose extension's differ.

App Store Connect asks about export compliance for each build until the app's `Info.plist` answers it with `ITSAppUsesNonExemptEncryption`, which needs a legal answer first.
