# Signing in

- The signed-out screens (`SignedOut` in `src/features/layout/RootLayout.tsx`) open with
  `DeploymentWelcome` (`src/features/auth/DeploymentWelcome.tsx`) above the sign-in or
  create-account form: the deployment's icon at 256 pixels (128 on a phone-sized screen, below `md`, so the
  forms' buttons stay in view), or Aspen's (`brand/aspen-icon.svg`) when it has none or
  its own fails to load, the welcome naming
  it (`welcomeNamed`), or `welcomeUnnamed` when it has no display name, both read from
  `GET /deployment` each time the screen opens (`AspenClient.deploymentProfile`), and the
  server's address. The welcome waits for the read rather than showing the general one and
  then the name; a failed read shows the general one. Which server the app signs in to is
  `defaultServerUrl` (`src/config.ts`): the web client always uses the origin serving the page,
  or the build's `VITE_ASPEN_SERVER_URL`, and never asks, so it offers no way to change the
  server (`ServerChoice.changeServer` is `null`); the desktop and mobile shells bake in none:
  they use the server the user entered, and until there is one ask with `ServerForm` ("Welcome
  to Aspen Chat." under Aspen's icon, above an empty Deployment URL field), keeping only
  the address's origin (`normalizeServerUrl`); the welcome offers to change it, which asks
  again from an empty field with a way back. `detectShell` tells the mobile shells apart by `Capacitor.isNativePlatform()`,
  since Android serves the bundle from `https://localhost`, which a browser tab can be too.
- Signing in: `AspenClient.login` answers `signedIn` or `secondFactorRequired` with a ticket,
  finished by `completeSecondFactor` or a passkey; `runPasskeyCeremony` runs any passkey
  ceremony (`signIn`, `register`, `reauthenticate`) end to end and stores the session a
  `signIn` produces. A ceremony reaches an authenticator by a `PasskeyTransport`
  (`packages/protocol/src/passkeys.ts`), chosen in `src/features/auth/passkeyTransport.ts`: the
  web client runs it in its own page when served under the server's `rpId` and offers no
  passkeys otherwise; the desktop and mobile shells hand it to the server's `/auth/passkey` page
  in the system browser, never running one in their own page. On the desktop the main process
  opens that page and listens on a one-shot loopback port for the browser's return
  (`packages/desktop/src/main/passkeyHandoff.ts`), then sends the tab back to the page to say
  it is done. On mobile the return is the `aspen://auth/passkey` URL, caught with
  `@capacitor/app` while `@capacitor/browser` shows the page, so the native projects register
  the `aspen` URL scheme (an intent filter on Android, `CFBundleURLTypes` on iOS once its
  project is made); this path has not yet run on a device.
- Signing in from another device (`OtherDeviceSignIn`, on the sign-in screen and as Other devices
  in Sign-in and security): a computer, the desktop app or a browser, shows a sign-in code
  (`DeviceLinkCode`) and a phone scans it, whichever of the two is signed in; phones never show
  one and computers never scan. Signed out, the computer's code asks for a sign-in and it claims
  one every couple of seconds (`AspenClient.claimDeviceLink`, which stores the session) until
  the phone confirms; signed in, its code offers the account, it asks every couple of seconds
  whether the code was scanned, and then asks the user to confirm the phone by the name the
  phone gave. A device names itself from its user agent alone (`thisDeviceName`: "Firefox on
  Linux", "Aspen on Android"), never a place. The code warns that it is a secret, counts down
  its minute, and then blurs with a button for a new one; a code still waiting when its screen
  closes is cancelled. The code holds `/device-link?server={origin}#{id}` under the
  deployment's web client (see QR codes), so a phone's own camera opens it in a browser too;
  the phone app's scanner reads it and opens the same route with the id as `link`.
  `DeviceLinkScreen` scans it once (a scan uses the code up, so a second render reuses the
  first's): signed in, it shows the computer's name with a warning that a stranger's code is a
  trap, and signs it in only on Sign it in; signed out, it says which account the phone will be
  signed in to and claims until the computer confirms. A code for another server is refused
  when signed in; signed out, a mobile shell moves to the code's server first
  (`ServerChoice.switchServer`), and the server form offers to scan a code instead of typing an
  address, since the code names its server. The root layout shows the screen in place of the
  sign-in forms while signed out (`/device-link`), and the router has it as a route signed in.
