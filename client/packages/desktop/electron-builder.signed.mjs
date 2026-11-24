// Packaging with our signature on it: `pnpm package:signed`, which CI runs for builds of `main`
// and of release tags and no pull request's, since whatever it builds carries the publisher's
// name. Everything else is `electron-builder.yml`. It is a module rather than YAML so that the
// Windows publisher comes from `signing.json`, beside the publishers the app's updater takes
// updates from (`src/main/updates.ts`), which must include it: an installed app refuses an update
// signed by anyone else, so a build signed by a publisher it does not name could never update
// the builds after it.
import { readFileSync } from "node:fs";
import { join } from "node:path";

const { windows } = JSON.parse(readFileSync(join(import.meta.dirname, "signing.json"), "utf8"));
if (!windows.updatesFrom.includes(windows.signer)) {
  throw new Error(
    `signing.json: ${windows.signer} signs, but updates are taken only from ${windows.updatesFrom.join(", ")}`,
  );
}

/** @type {import("electron-builder").Configuration} */
export default {
  extends: "./electron-builder.yml",
  // Marks the packaged `package.json` of the build that updates itself (`src/main/updates.ts`):
  // only a signed build can, its updates being checked for the same signature.
  extraMetadata: { aspen: { updates: true } },
  win: {
    // Without this a build whose signing failed would still produce an installer.
    forceCodeSigning: true,
    // Azure Trusted Signing, which keeps the key and signs what the service principal in
    // `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, and `AZURE_CLIENT_SECRET` sends it. Its certificates
    // last days, not years, so a signature holds only by its timestamp, which electron-builder
    // asks Microsoft's timestamp service for unless told otherwise.
    azureSignOptions: {
      // The certificate's common name, which Windows shows as the publisher.
      publisherName: windows.signer,
      endpoint: "https://eus.codesigning.azure.net/",
      codeSigningAccountName: "AspenChat",
      certificateProfileName: "AspenChat",
    },
  },
  mac: {
    // The Developer ID Application identity is the `.p12` in `CSC_LINK` (its password
    // `CSC_KEY_PASSWORD`). CI gives no notarization key (`APPLE_API_KEY` and the rest), so
    // electron-builder skips notarizing, with a warning: CI signs each dmg and sends it to the
    // notary service itself, and `.github/workflows/notarized-dmg.yml` staples it once accepted.
    forceCodeSigning: true,
  },
};
