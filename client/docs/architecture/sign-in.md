# Signing in

## Where it lives

| Part                           | Where                                                                                                                                                                  |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Signed-out screens             | `SignedOut` in `src/features/layout/RootLayout.tsx`                                                                                                                    |
| The welcome                    | `DeploymentWelcome` (`src/features/auth/DeploymentWelcome.tsx`)                                                                                                        |
| Which server                   | `defaultServerUrl` (`src/config.ts`), `ServerChoice`, `ServerForm`, `normalizeServerUrl`, `detectShell`                                                                |
| Passkeys                       | `runPasskeyCeremony`, `PasskeyTransport` (`packages/protocol/src/passkeys.ts`), `src/features/auth/passkeyTransport.ts`, `packages/desktop/src/main/passkeyHandoff.ts` |
| Passkey autofill and signals   | `useConditionalPasskey`, `passkeySignals.ts` (`src/features/auth`), `runConditionalPasskeySignIn`                                                                      |
| Android's own passkeys         | `nativePasskeys.ts` (`src/features/auth`), `AspenPasskeysPlugin` (`packages/mobile/android`)                                                                           |
| Signing in from another device | `OtherDeviceSignIn`, `DeviceLinkCode`, `DeviceLinkScreen`, `thisDeviceName`                                                                                            |
| Email                          | `src/features/email`: `useEmailPolicy`, `PasswordReset`, `VerificationScreen`, `EmailDialog`, `EmailPanel`                                                             |
| Device credentials             | `AspenClient` and `device.ts` (`packages/protocol/src`), `src/api/devices.ts`, `src/api/devicePuzzle.worker.ts`                                                        |

## Device credentials

Every request made before signing in presents the deployment's device credential (`Aspen-Device`),
which is what the server counts it by: many people on one network are then each counted on their
own.

- `AspenClient` earns one when a server answers `deviceCredentialRequired`, then sends the request
  again. Requests refused together share one earning.
- Earning one means solving a puzzle of about a million SHA-256 hashes, a second or two on a phone.
  `solveInWorker` runs it in a worker so the screen does not freeze, and on the page's thread, a
  slice at a time, where workers cannot run.
- The first request a new install makes waits for it: the welcome's read of `GET /deployment`, or
  `GET /auth/methods`.
- It is kept per deployment in `localStorage` (`aspen.device.<host>`), outlives signing out, and is
  replaced when a server answers with a fresh one (`Aspen-Device-Credential`).
- Signed-in requests carry the session instead.

How the server makes and checks them: `docs/architecture/sign-in-and-security/device-credentials.md`
at the repository's root.

## The signed-out screens

The signed-out screens open with `DeploymentWelcome` above the sign-in or create-account form.

### The welcome

It shows:

- the deployment's icon at 256 pixels, or 128 on a phone-sized screen (below `md`), so the forms'
  buttons stay in view;
- Aspen's icon (`brand/aspen-icon.svg`) when the deployment has none or its own fails to load;
- the welcome naming it (`welcomeNamed`), or `welcomeUnnamed` when it has no display name;
- the server's address.

- The icon and name are read from `GET /deployment` each time the screen opens
  (`AspenClient.deploymentProfile`).
- The welcome waits for the read rather than showing the general one and then the name.
- A failed read shows the general one.

### Which server

`defaultServerUrl` (`src/config.ts`) decides which server the app signs in to.

| Shell              | Server                                                                                                                                                              | Changing it                                                                           |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| Web client         | Always the origin serving the page, which is the deployment's (its API and web client share one origin). It never asks                                              | Not offered (`ServerChoice.changeServer` is `null`)                                   |
| Desktop and mobile | None baked in. The server the user entered. Until there is one, `ServerForm` asks: "Welcome to Aspen Chat." under Aspen's icon, above an empty Deployment URL field | The welcome offers to change it, which asks again from an empty field with a way back |

- `ServerForm` keeps only the address's origin (`normalizeServerUrl`).
- `detectShell` tells the mobile shells apart by `Capacitor.isNativePlatform()`, since Android
  serves the bundle from `https://localhost`, which a browser tab can be too.

## Signing in

`AspenClient.login` answers one of:

- `signedIn`;
- `secondFactorRequired`, with a ticket. That is finished by `completeSecondFactor` or a passkey.

### Passkeys

`runPasskeyCeremony` runs any passkey ceremony end to end: `signIn`, `register`, or
`reauthenticate`. It stores the session a `signIn` produces.

A ceremony reaches an authenticator by a `PasskeyTransport` (`packages/protocol/src/passkeys.ts`),
chosen in `src/features/auth/passkeyTransport.ts`:

| Shell                                       | Transport                                                                                                |
| ------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| Web client served under the server's `rpId` | Runs the ceremony in its own page                                                                        |
| Web client served elsewhere                 | Offers no passkeys                                                                                       |
| Desktop and mobile                          | Hands it to the server's `/auth/passkey` page in the system browser, never running one in their own page |
| Android, where the deployment names the app | Runs it through Credential Manager, falling back to the system browser                                   |

On the desktop (`packages/desktop/src/main/passkeyHandoff.ts`):

1. The main process opens that page.
2. It listens on a one-shot loopback port for the browser's return.
3. It sends the tab back to the page to say it is done.

On mobile:

1. `@capacitor/browser` shows the page.
2. The return is the `aspen://auth/passkey` URL, caught with `@capacitor/app`.

- The native projects register the `aspen` URL scheme: an intent filter on Android,
  `CFBundleURLTypes` in `ios/App/App/Info.plist` on iOS, whose `SceneDelegate` hands opened URLs to
  Capacitor (`SceneDelegateProxy`).
- This path has not yet run on a device.

### Android's own passkeys

A deployment that names the Android app in its `/.well-known/assetlinks.json` lets the app use its
passkeys through Credential Manager, as a page at its domain would (see the server's
`docs/architecture/sign-in-and-security/passkeys.md`). `resolvePasskeyTransport` picks the `native`
transport when:

- the shell is the Android app with `AspenPasskeysPlugin`, on Android 9 or later;
- `GET /auth/methods` lists the app's package with a certificate it is signed with
  (`passkeys.androidApps`; a deployment that predates the list names none).

The plugin hands Credential Manager the ceremony's `publicKey` options as the server gave them, and
the credential back as the JSON the server reads. It asks only for passkeys and providers at hand on
the phone (`preferImmediatelyAvailableCredentials`): otherwise, with none for the deployment,
Google's sheet waits to be dismissed, which reads as cancelled, where the browser could still have
reached a security key or another phone. `PasskeyPlatformTest` (a device test) holds that a
deployment that does not name the app fails at once as `unavailable`. The ceremony starts and
completes as one in a page does, by the sign-in that started it. When Credential Manager is missing,
no provider answers, or Android refuses the app for the domain (`unavailable`), the ceremony is left
to expire and another goes to the system browser (`fallback`). A dismissed prompt (`cancelled`) ends
it.

### Autofill

The web client's sign-in form offers passkeys among the username field's suggestions
(`autocomplete="username webauthn"`), where it runs ceremonies in its own page and the browser has
conditional mediation. `useConditionalPasskey` starts a `signIn` ceremony as the form opens
(`runConditionalPasskeySignIn`, `mediation: "conditional"`), and the one picked signs in.

- Signing in with a password, pressing the passkey button, or leaving the form aborts it first.
- It ends 15 seconds before its ceremony expires.
- Another starts only when the person returns to the username field with none waiting, never on a
  timer: each start counts toward the sign-in rate limits.
- A refused start is silent. A passkey the server refuses shows its Problem, as the button's does.

### Signals

Where the page runs ceremonies itself (`inPage`), it tells the browser's passkey managers what the
server holds, through WebAuthn's Signals API (`signalUnknownCredential`,
`signalAllAcceptedCredentials`, `signalCurrentUserDetails` in `packages/protocol/src/passkeys.ts`),
feature-detected and silent where missing:

- a passkey the server answers `passkeyUnknown` for, from the ceremony that used it;
- every passkey of the account, each time Sign-in and security reads its settings
  (`useSignalAcceptedPasskeys`), so after every change there too;
- the username and display name after the profile's display name changes (`signalProfileToPasskeys`,
  which reads the account's user handle from its security settings only where the browser has the
  signal, and sends nothing for an account without passkeys).

## Signing in from another device

`OtherDeviceSignIn` is on the sign-in screen, and as Other devices in Sign-in and security.

- A computer (the desktop app or a browser) shows a sign-in code (`DeviceLinkCode`).
- A phone scans it.
- Either of the two may be the one signed in.
- Phones never show a code, and computers never scan.

### The computer's code

| Computer is | The code           | Then                                                                                                                             |
| ----------- | ------------------ | -------------------------------------------------------------------------------------------------------------------------------- |
| Signed out  | Asks for a sign-in | It claims one every couple of seconds (`AspenClient.claimDeviceLink`, which stores the session) until the phone confirms         |
| Signed in   | Offers the account | It asks every couple of seconds whether the code was scanned, then asks the user to confirm the phone by the name the phone gave |

- A device names itself from its user agent alone (`thisDeviceName`: "Firefox on Linux", "Aspen on
  Android"), never a place.
- The code warns that it is a secret and counts down its minute. Then it blurs, with a button for a
  new one.
- A code still waiting when its screen closes is cancelled.
- The code holds `/device-link?server={origin}#{id}` under the deployment's web client (see
  [qr-codes.md](qr-codes.md)). So a phone's own camera opens it in a browser too.
- The phone app's scanner reads it and opens the same route with the id as `link`.

### The phone's screen

`DeviceLinkScreen` scans the code once. A scan uses the code up, so a second render reuses the
first's.

| Phone is   | Shows                                                                | Then                               |
| ---------- | -------------------------------------------------------------------- | ---------------------------------- |
| Signed in  | The computer's name, with a warning that a stranger's code is a trap | Signs it in only on Sign it in     |
| Signed out | Which account the phone will be signed in to                         | Claims until the computer confirms |

### A code for another server

- Signed in, it is refused.
- Signed out, an app first asks whether to move to the code's server. It names the host and warns
  that only a code the person made themselves is safe, since anyone can make a link naming any
  server.
- It moves (`ServerChoice.switchServer`) only on Switch. Nothing is asked of that server before
  then.

### From the server form

The server form offers to scan a code instead of typing an address, since the code names its server.

- A scanned code, or one opened as a link (`aspen://app/device-link`) before any server is chosen,
  fills the address in and says so.
- The person continues to it themselves.

### Routing

- Signed out, the root layout shows the screen in place of the sign-in forms (`/device-link`).
- Signed in, the router has it as a route.

## Email

Code: `src/features/email`.

What the deployment does with email is `email` in `GET /deployment` (`useEmailPolicy`), read each
time a screen offering it opens. A deployment that predates it lacks it, which is taken to send
none.

### Creating an account

Where the deployment sends mail, the create-account form:

- asks for an address, required where `required`;
- once one is typed, and where the deployment has a newsletter, offers it with an unchecked "Send me
  the newsletter".

### Resetting a password

The sign-in form offers "Forgot your password?" (`PasswordReset`):

1. the username;
2. the account's address, typed whole, beside the server's masked one;
3. the code mailed there, with a new password;
4. back to signing in, with a notice.

A reset the server ended (expired, used up, too many wrong tries) starts again from the username
with its reason.

### Required verification

A deployment requiring a verified address:

- answers every other request of an account whose address is not verified with
  `emailVerificationRequired`;
- closes its event stream with 4428.

`AspenClient` flags the session (`emailVerificationRequired`, `noticeVerificationRequired`), as it
does a second factor owed. The root layout then shows `VerificationScreen`, after the enrollment
screen if both are owed, until either:

- the code mailed to the address is typed; or
- the address is changed and its code typed (`markEmailVerified`).

That mounts the app afresh.

### Email settings

Signed in, Settings' Email (`EmailDialog`) shows the same `EmailPanel`. It is offered to a person of
this deployment, where it sends mail.

The panel holds:

- the address, added, changed, or removed. Each goes through `useReauth`, since a verified address
  can reset the password. Removal is not offered where the deployment requires one;
- its verification;
- what the address receives:
  - whether the profile shows it;
  - the newsletter, offered where the deployment has one, or while subscribed;
  - the daily digest, with its time zone (this device's when first turned on, then any the browser
    knows) and hour.

The panel reads `GET /users/@me/email` on opening, and again on each `emailAccountChanged` event
(`RecordStore.emailChanges`, topic `email`).
