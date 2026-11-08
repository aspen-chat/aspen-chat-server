# The age gate

Aspen is for adults. Before anything else of the app shows, the iOS and Android apps ask the device
whether its user is under 18. Where it says so, all the app shows is:

> Your device has indicated you are not old enough to use this app. We do not store your age in any
> location. This determination is made by your operating system and legal jurisdiction.

## Where it lives

| Part     | Where                                                                              |
| -------- | ---------------------------------------------------------------------------------- |
| The gate | `AgeGate` (`src/features/layout/AgeGate.tsx`), around `App` in `main.tsx`          |
| Asking   | `canAskAge`, `deviceReportsUnderage` (`src/api/ageGate.ts`)                        |
| iOS      | `AspenAgePlugin.swift` (`packages/mobile/ios/App/App`), Declared Age Range         |
| Android  | `AspenAgePlugin.java` (`packages/mobile/android`, package `age`), Play Age Signals |

## What decides it

- **iOS** asks only where the user's region requires age assurance (`isEligibleForAgeFeatures`, iOS
  26.2 and later), so nobody elsewhere sees the system's sheet. It then asks for an age range with
  one gate, 18 (`requestAgeRange`). A range whose upper bound is below 18 is underage. The first
  question has five seconds to be answered (`eligibilityWaitNanoseconds`), after which the app goes
  on as where no assurance is required. **Why:** it has been seen never to be answered, and the app
  shows nothing until it is. The sheet a person answers is never timed.
- **Android** asks Play Age Signals (`com.google.android.play:age-signals`). Play answers with an
  age band only where the law has it, and only for a user who shares one. A band whose upper bound
  is below 18 (with Play's default bands, 12, 15, or 17) is underage. The highest band has no upper
  bound.
- **Anything else lets the user in:** a device that reports no age, a user who declines to share, an
  iOS too old to ask, an iOS that does not say in time whether to ask, an app not installed from
  Play, and any failure. The web client and the desktop app cannot ask, and show the app at once.

## What is kept

Nothing. The answer is held in memory for the launch (`deviceReportsUnderage` asks once however
often it is wanted) and asked again the next time the app starts. It is never written to storage and
never sent to a deployment.

## What a build needs

- iOS: the `com.apple.developer.declared-age-range` entitlement (`App.entitlements`), so the App ID
  and its provisioning profiles need the Declared Age Range capability.
- Android: nothing in the Play Console; Play Age Signals answers apps installed from Play.

## Testing it

Apple supports Declared Age Range only on devices; on a simulator it leaves the app waiting. A
simulator build never calls it, and answers from the launch argument `-AspenAgeCheck`: `underage`
shows the notice, and anything else, or none, lets the user in. Devices never read it. The UI test
`testShowsOnlyTheNoticeToAnUnderageUser` launches with `underage` (`ios-app.md`).

The real check is tried on a device running iOS 26.2 or later, signed in to a Sandbox Apple Account
in Settings → Developer → Sandbox Testing alone, whose Age Assurance page sets the region and the
age range the system answers with.
