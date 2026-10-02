import { RouterProvider } from "@tanstack/react-router";
import { useState } from "react";
import { AspenProvider } from "@/api/AspenProvider";
import { defaultServerUrl, rememberServerUrl } from "@/config";
import { ServerForm } from "@/features/auth/ServerForm";
import { ServerChoiceContext } from "@/features/auth/serverChoice";
import { Toasts } from "@/features/layout/Toasts";
import { router } from "@/router";

/**
 * Chooses the server, then hands the page to the router. Everything the router renders lives
 * inside `<AspenProvider>`, so the client for the chosen server is available to every route.
 */
export function App() {
  const [serverUrl, setServerUrl] = useState<string | null>(() => defaultServerUrl());
  const [choosingServer, setChoosingServer] = useState(false);

  if (serverUrl === null || choosingServer) {
    return (
      <main className="flex min-h-full items-center justify-center p-6">
        <ServerForm
          initial={serverUrl ?? ""}
          onSubmit={(url) => {
            rememberServerUrl(url);
            setServerUrl(url);
            setChoosingServer(false);
          }}
        />
      </main>
    );
  }

  return (
    <AspenProvider serverUrl={serverUrl}>
      <ServerChoiceContext.Provider
        value={{
          serverUrl,
          changeServer: () => {
            setChoosingServer(true);
          },
        }}
      >
        <RouterProvider router={router} />
        <Toasts fallback />
      </ServerChoiceContext.Provider>
    </AspenProvider>
  );
}
