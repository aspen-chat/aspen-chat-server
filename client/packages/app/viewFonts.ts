import type { Plugin } from "vite";

/** Where the web client serves `src/viewFonts.css`, under a name that never changes. */
export const VIEW_FONTS = "view-fonts.css";
const SOURCE = "src/viewFonts.css";

/**
 * Plugins' views and the app's fonts. A view is a page of its plugin's, served by the deployment
 * in a sandbox with an origin of its own, so it cannot load the app's stylesheets or the faces
 * the user added (`theme/fontLibrary.ts`); the bridge hands it the font stacks, the address of
 * every face the app bundles, and the user's faces as files (`features/plugins/PluginChannel`).
 *
 * This plugin builds `src/viewFonts.css` as an entry of its own, written to `view-fonts.css` at
 * the web client's root, which the deployment serves beside the app (`api::web_client`) and
 * whose faces' files are the app's own, under `assets/`. A view's page is in no origin it could
 * share a font with, so its fonts are fetched with CORS, which the deployment allows for every
 * origin. In `vite dev` the same name is answered with the stylesheet as Vite serves it, and the
 * font files it names with `Access-Control-Allow-Origin` too, which Vite otherwise gives only
 * pages of its own origin.
 */
export function viewFonts(): Plugin {
  return {
    name: "aspen-view-fonts",
    config: () => ({
      build: {
        rollupOptions: {
          input: { index: "index.html", viewFonts: SOURCE },
          output: {
            assetFileNames: (asset) =>
              asset.names.includes("viewFonts.css") ? VIEW_FONTS : "assets/[name]-[hash][extname]",
          },
        },
      },
    }),
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const path = request.url?.split("?")[0] ?? "";
        if (path === `/${VIEW_FONTS}`) {
          request.url = `/${SOURCE}?direct`;
          response.setHeader("Access-Control-Allow-Origin", "*");
        } else if (/\.(woff2?|ttf|otf)$/.test(path)) {
          response.setHeader("Access-Control-Allow-Origin", "*");
        }
        next();
      });
    },
  };
}
