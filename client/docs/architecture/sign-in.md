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
