import { RouterProvider } from "@tanstack/react-router";
import { useState } from "react";
import { AspenProvider } from "@/api/AspenProvider";
import { useOpenAppLinks } from "@/api/appLinks";
import { defaultServerUrl, detectShell, rememberServerUrl } from "@/config";
import { ServerForm } from "@/features/auth/ServerForm";
import { ServerChoiceContext } from "@/features/auth/serverChoice";
import { Toasts } from "@/features/layout/Toasts";
import { router } from "@/router";

/**
 * Chooses the server (`defaultServerUrl`, or the user's answer to `ServerForm` in a shell with
 * none), then hands the page to the router. Everything the router renders lives
 * inside `<AspenProvider>`, so the client for the chosen server is available to every route.
 */
export function App() {
  const [serverUrl, setServerUrl] = useState<string | null>(() => defaultServerUrl());
  const [choosingServer, setChoosingServer] = useState(false);
  const [offeredServer, setOfferedServer] = useState<string | null>(null);

  function choose(url: string) {
    rememberServerUrl(url);
    setServerUrl(url);
    setChoosingServer(false);
  }

  // A sign-in code opened before any server is chosen names the one to use, which the server
  // form offers rather than choosing it: a link can name any server.
  useOpenAppLinks((link) => {
    if (serverUrl === null) {
      setOfferedServer(link.server);
    }
  });

  if (serverUrl === null || choosingServer) {
    return (
      <main className="flex min-h-full items-center justify-center p-6">
        <ServerForm
          onSubmit={choose}
          offered={serverUrl === null ? offeredServer : null}
          {...(serverUrl === null
            ? {}
            : {
                onCancel: () => {
                  setChoosingServer(false);
                },
              })}
        />
      </main>
    );
  }

  return (
    <AspenProvider serverUrl={serverUrl}>
      <ServerChoiceContext.Provider
        value={{
          serverUrl,
          changeServer:
            detectShell() === "web"
              ? null
              : () => {
                  setChoosingServer(true);
                },
          switchServer: detectShell() === "web" ? null : choose,
        }}
      >
        <RouterProvider router={router} />
        <Toasts fallback />
      </ServerChoiceContext.Provider>
    </AspenProvider>
  );
}
