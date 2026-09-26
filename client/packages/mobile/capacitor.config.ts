import type { CapacitorConfig } from "@capacitor/cli";

const config: CapacitorConfig = {
  appId: "org.aspenchat.client",
  appName: "Aspen",
  // The renderer is the @aspen/app production build; `pnpm sync` builds it and copies it in.
  webDir: "../app/dist",
  server: {
    // Serve the bundle from an `https://` origin on Android so the page is a secure context
    // (required for several web APIs) and matches iOS's `capacitor://localhost` in privilege.
    androidScheme: "https",
  },
};

export default config;
