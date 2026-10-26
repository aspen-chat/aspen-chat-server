# Signing in

## Where it lives

| Part                           | Where                                                                                                                                                                  |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Signed-out screens             | `SignedOut` in `src/features/layout/RootLayout.tsx`                                                                                                                    |
| Deleting the account           | `DeleteAccountSection` in `src/features/security/SecurityPanel.tsx`, `deleteAccount` (`src/features/security/api.ts`)                                                  |
| The welcome                    | `DeploymentWelcome` (`src/features/auth/DeploymentWelcome.tsx`)                                                                                                        |
| Which server                   | `defaultServerUrl` (`src/config.ts`), `ServerChoice`, `ServerForm`, `normalizeServerUrl`, `detectShell`                                                                |
| Passkeys                       | `runPasskeyCeremony`, `PasskeyTransport` (`packages/protocol/src/passkeys.ts`), `src/features/auth/passkeyTransport.ts`, `packages/desktop/src/main/passkeyHandoff.ts` |
| Passkey autofill and signals   | `useConditionalPasskey`, `passkeySignals.ts` (`src/features/auth`), `runConditionalPasskeySignIn`                                                                      |
| Android's own passkeys         | `nativePasskeys.ts` (`src/features/auth`), `AspenPasskeysPlugin` (`packages/mobile/android`)                                                                           |
| Signing in from another device | `OtherDeviceSignIn`, `DeviceLinkCode`, `DeviceLinkScreen`, `thisDeviceName`                                                                                            |
| Email                          | `src/features/email`: `useEmailPolicy`, `PasswordReset`, `VerificationScreen`, `EmailDialog`, `EmailPanel`                                                             |
| Device credentials             | `AspenClient` and `device.ts` (`packages/protocol/src`), `src/api/devices.ts`, `src/api/devicePuzzle.worker.ts`                                                        |
| Identity providers             | `identityProviders.ts` (`packages/protocol/src`), `src/features/auth`: `identityProviders.ts`, `IdentityProviderButtons`, `ProviderReturn`, `ProviderAccountStep`      |

## Device credentials

Every request made before signing in presents the deployment's device credential (`Aspen-Device`),
which is what the server counts it by: many people on one network are then each counted on their
own.

- `AspenClient` earns one when a server answers `deviceCredentialRequired`, then sends the request
  again. Requests refused together share one earning.
- Earning one means solving a puzzle of about a million SHA-256 hashes, a second or two on a phone.
  `solveInWorker` runs it in a worker so the screen does not freeze, and on the page's thread, a
  slice at a time, where workers cannot run.
- A deployment cannot make it work without end. A puzzle harder than `MAX_PUZZLE_BITS` (28) is not
  started, and one not solved in `PUZZLE_TIME_LIMIT_MS` (a minute) is stopped, ending the worker.
  The requests waiting on it then fail with `DevicePuzzleRefusedError`, as if the deployment could
  not be reached, so the earning always settles.
- The first request a new install makes waits for it: the welcome's read of `GET /deployment`, or
  `GET /auth/methods`.
- It is kept per deployment in `localStorage` (`aspen.device.<host>`), outlives signing out, and is
  replaced when a server answers with a fresh one (`Aspen-Device-Credential`).
- Signed-in requests carry the session instead.

How the server makes and checks them: `docs/architecture/sign-in-and-security/device-credentials.md`
at the repository's root.

## The signed-out screens

The signed-out screens open with `DeploymentWelcome` above the sign-in or create-account form, and
end with links to the deployment's Privacy Policy and Terms of Use (`documents.md`).

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
2. It listens on a one-shot loopback port for the browser's return, answering only a return that
   names the ceremony the page was opened for (the `ceremony` in its fragment) and refusing any
   other request, so a local page or process that finds the port cannot settle the hand-off first.
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
the credential back as the JSON the server reads, but only when the options' relying party (`rpId`
to sign in, `rp.id` to register) is the deployment's host or a parent of it (`rpIdFits`, the rule
`canRunInPage` holds a page to). `AspenClient.runPasskeyCeremony` checks it before calling the
plugin, which is told the host and checks it again; anything else goes to the system browser, as
`unavailable` does. Credential Manager checks only that the relying party's domain names the app,
and the app's origin is the same for every deployment, so without it a deployment could relay
another's ceremony and have the app sign it. It asks only for passkeys and providers at hand on the
phone (`preferImmediatelyAvailableCredentials`): otherwise, with none for the deployment, Google's
sheet waits to be dismissed, which reads as cancelled, where the browser could still have reached a
security key or another phone. `PasskeyPlatformTest` (a device test) holds that a deployment that
does not name the app fails at once as `unavailable`. The ceremony starts and completes as one in a
page does, by the sign-in that started it. When Credential Manager is missing, no provider answers,
or Android refuses the app for the domain (`unavailable`), the ceremony is left to expire and
another goes to the system browser (`fallback`). A dismissed prompt (`cancelled`) ends it.

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
- Beside the QR code is a user code (`BCDF-GHJK`). Any device, a computer included, may type it
  instead of scanning (`TypeDeviceLinkCode`, Type a code instead).

### The computer's code

| Computer is | The code           | Then                                                                                                                             |
| ----------- | ------------------ | -------------------------------------------------------------------------------------------------------------------------------- |
| Signed out  | Asks for a sign-in | It claims one every couple of seconds (`AspenClient.claimDeviceLink`, which stores the session) until the phone confirms         |
| Signed in   | Offers the account | It asks every couple of seconds whether the code was scanned, then asks the user to confirm the phone by the name the phone gave |

- A device names itself from its user agent alone (`thisDeviceName`: "Firefox on Linux", "Aspen on
  Android"), never a place.
- The code warns that it is a secret and counts down its five minutes. Then it blurs, with a button
  for a new one.
- A code still waiting when its screen closes is cancelled.
- The code holds `/device-link?server={origin}#{id}` under the deployment's web client (see
  [qr-codes.md](qr-codes.md)). So a phone's own camera opens it in a browser too.
- The phone app's scanner reads it and opens the same route with the id as `link`.

### Confirming a typed request's account

A signed-in device that typed a computer's code may have guessed it. So when the computer's claim
answers `confirmAccount`, it shows the account it would be signed in to (picture, display name, and
username) and a warning, and claims it (`claimDeviceLink` with `account`) only on This is my
account. Don't sign in cancels the code.

### The phone's screen

`DeviceLinkScreen` scans the code once. A scan uses the code up, so a second render reuses the
first's (`takenLinks.ts`).

| Phone is   | Shows                                                                | Then                               |
| ---------- | -------------------------------------------------------------------- | ---------------------------------- |
| Signed in  | The computer's name, with a warning that a stranger's code is a trap | Signs it in only on Sign it in     |
| Signed out | Which account the phone will be signed in to                         | Claims until the computer confirms |

### Typing a code

`TypeDeviceLinkCode` is a button beside the scanner on a phone, and beside the code's button on a
computer. It opens a dialog with one field.

- The code is read as the server reads it (`normalizeUserCode`: any case, hyphens and spaces
  dropped, full-width letters as letters). Input that cannot be a code is refused at the field and
  not sent, since the server counts every code that matches nothing against whoever typed it.
- A signed-in device verifies again first when the server asks (`useOptionalReauth`).
- Once taken, the dialog shows `DeviceLinkScreen` for the link, as a scanned code's route does, but
  is left by closing the dialog rather than for `/`. A signed-in giver is then told that the other
  device confirms the account to finish.

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

## Identity providers

People may sign in through the OpenID Connect providers the deployment's administrators chose (the
server's `docs/architecture/sign-in-and-security/identity-providers.md`). `GET /auth/methods` lists
them (`offeredProviders`; a deployment that predates them lists none), and the sign-in screen has a
"Continue with {name}" button for each (`IdentityProviderButtons`), below the passkey button.

Every exchange with a provider is a flow, of a purpose: `signIn`, `link`, or `reauthenticate`.
`useProviderFlow` runs one. The client starts it with a PKCE challenge of its own
(`AspenClient.startIdentityProviderFlow`), the browser signs in at the provider and comes back with
the flow's id and a one-time return code, and the client claims what the provider said with its
verifier and that code (`claimIdentityProviderFlow`).

### Where the browser goes

| Shell              | The provider's sign-in                                                                      | The return                                                                                                        |
| ------------------ | ------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Web client         | This tab. The verifier is kept in `sessionStorage` by the flow's id (`keepFlow`)            | `/sign-in/provider#flow={id}&code={code}`, a route of its own (`ProviderReturn`)                                  |
| Desktop and mobile | The system browser, through the passkey hand-off's machinery (`runIdentityProviderHandoff`) | The hand-off's return address, with `flow` and `code` in the query, read as a finished ceremony named by the flow |

In the web client:

- The tab goes to the provider's page only over HTTPS (`requireSecure`, which allows this machine's
  own addresses in a development build), whatever address the deployment named.
- The tab remembers where it was (`returnPath`), and lands there when the flow is done.
- A flow is taken from storage once (`takeFlow`), and flows older than the server keeps them are
  dropped as another starts (`FLOW_KEPT_MS`).
- The return page takes the fragment off the address before claiming, and claims each flow once
  however often it is drawn. Signed out, the root layout shows it in place of the sign-in forms.
- A return this tab did not start says so, with the way back to signing in.

In the apps (`providerHandoff`):

- The desktop prepares a passkey hand-off's loopback listener and opens the provider instead of the
  passkey page (`passkey:open-provider`, `openProvider` in
  `packages/desktop/src/main/passkeyHandoff.ts`), which takes only HTTPS (or this machine's own
  addresses in an unpackaged build). The listener answers only a return whose `flow` is the flow's
  id (the authorization URL's `state`), reads its `code`, and sends the tab on to the server's
  passkey page to say it is done.
- A phone uses the passkey hand-off as it is: `aspen://auth/passkey`, read by `parseHandoffReturn`.
  Its in-app browser opens a passkey page or a provider's page only over HTTPS (`requireSecure`).
- Closing the browser, or the hand-off giving up, is backing out (`PasskeyCancelledError`), which
  shows nothing.

### What a claim does

| Outcome                | Then                                                                                                               |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `signedIn`             | The session is stored, and the signed-in app opens                                                                 |
| `secondFactorRequired` | `SecondFactorStep`, as after a password                                                                            |
| `accountRequired`      | `ProviderAccountStep`, below                                                                                       |
| `linked`               | Said in a toast. The apps reload the security settings; the web client returns to where it was                     |
| `reauthenticated`      | The apps finish the change that asked; the web client returns to where it was and asks in a toast to make it again |
| A Problem              | Its text, as the server gave it: `identityProviderFailed`'s detail says why the provider did not sign them in      |

### A new account

`ProviderAccountStep` asks for a username, prefilled with the provider's (`suggestedName`), and
makes the account with the same verifier (`createIdentityProviderAccount`).

- It asks for a registration invite only where the offer says one is needed (`inviteRequired`).
- It names the display name and the address the account will have, when the provider gave them.
- Without an address from the provider, it asks for one as registration does: required where the
  offer says (`emailRequired`), and optional where the deployment sends mail.
- The newsletter is offered as registration offers it.
- A username someone holds (`usernameTaken`) shows at the field, and another may be tried.
- "Sign in another way" goes back to signing in.

### Security settings

Sign-in and security (`SecurityPanel`) shows, for a person of this deployment:

- the identity providers linked to the account, each with when it was linked and last used, and an
  unlink button. Unlinking needs a recent verification, and the server refuses the account's only
  way in (`lastSignInMethod`), which shows as the section's error;
- a "Link {name}" button for each provider the server offers that is not linked, running a `link`
  flow, which needs a recent verification before it starts;
- for an account without a password (`hasPassword` false), "Set a password" in place of changing it,
  without the current one, after verifying again.

A server that predates providers leaves out `hasPassword` and `identities`, read as a password and
none (`hasPassword`, `linkedIdentities` in `src/features/security/api.ts`).

### Deleting the account

Sign-in and security ends with Delete your account, for a person of this deployment (a user of
another deletes their account at home, which tells the deployments they used):

- It asks first, saying what deleting does: every sign-in ends; the email address, password,
  passkeys, and second factors go with the account; owned communities pass to their highest-ranked
  members; messages stay where they were posted.
- It deletes with `DELETE /users/@me`, which needs a recent verification (`useReauth`).
- Once deleted, the app signs out at home and everywhere it signed in from there (`useSignOut`). The
  server has already ended those sign-ins, and tells the other deployments the account used.

### Verifying again

The verify-again dialog (`ReauthProvider`) offers "Confirm with {name}" for each linked provider, a
`reauthenticate` flow. For an account without a password it asks for no password: a passkey or a
provider confirms it. In the web client the tab goes to the provider, so the change that asked is
made again on returning; in the apps it finishes in place.

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
