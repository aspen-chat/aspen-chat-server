import type { CapacitorConfig } from "@capacitor/cli";

const config: CapacitorConfig = {
  // The Android application's id. The Xcode project names the iOS bundle itself
  // (`com.aspen-chat.client`): Android takes no hyphen in an id, and Apple no underscore.
  appId: "com.aspen_chat.client",
  appName: "Aspen",
  // The renderer is the @aspen/app production build; `pnpm sync` builds it and copies it in.
  webDir: "../app/dist",
  // Copies the web view's console to the system's log (os_log, logcat) in Release builds too,
  // which `cap sync` writes into the native projects. Only builds for the developers' own
  // devices set `ASPEN_DEVICE_LOGS=1`: the console names servers and what the app did there,
  // which a published build keeps off its users' system logs.
  loggingBehavior: process.env.ASPEN_DEVICE_LOGS === "1" ? "production" : "debug",
  server: {
    // Serve the bundle from an `https://` origin on Android so the page is a secure context
    // (required for several web APIs) and matches iOS's `capacitor://localhost` in privilege.
    androidScheme: "https",
  },
};

export default config;
